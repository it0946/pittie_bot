use anyhow::Context;
use futures::stream::StreamExt;
use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    fs,
    io::{BufReader, Read},
    path::PathBuf,
    process::ExitCode,
    sync::{Arc, RwLock},
};
use twilight_gateway::{cluster::Events, Cluster, Event};
use twilight_http::Client as HttpClient;
use twilight_model::{
    gateway::{payload::incoming::MessageCreate, Intents},
    http::attachment::Attachment,
    id::{marker::UserMarker, Id},
};

type Image = (String, Vec<u8>);
type Images = RwLock<Vec<(String, Vec<u8>)>>;
type HandlerError = Result<(), Box<dyn Error + Send + Sync>>;

const IMAGES_DIR: &str = "./images";
const CONFIG_PATH: &str = "./pittie_config.json";

lazy_static::lazy_static! {
    static ref CONFIG: Config = Config::init_config(CONFIG_PATH);
}

struct EventHandler {
    http: HttpClient,
    images: Images,
}

impl EventHandler {
    fn new(http: HttpClient, images: Images) -> Arc<Self> {
        Arc::new(Self { http, images })
    }

    fn get_random_image(&self) -> Option<Image> {
        Some({
            let read = self
                .images
                .read()
                .expect("failed to acquire images read lock");

            if read.is_empty() {
                return None;
            }

            let i = fastrand::usize(..read.len());
            (read[i].0.clone(), read[i].1.clone())
        })
    }

    fn get_rand_attachment(&self) -> Option<Attachment> {
        if let Some((filename, file)) = self.get_random_image() {
            Some(Attachment {
                description: None,
                file,
                filename,
                id: 0,
            })
        } else {
            None
        }
    }

    // TODO
    async fn process_cmd(self: Arc<Self>, msg: Box<MessageCreate>) -> HandlerError {
        let args = &msg.content.as_str()[CONFIG.prefix.len()..];
        let args: Vec<&str> = args.split(" ").collect();

        match args[0] {
            "pittie" => self.cmd_pittie(&msg).await,
            // can't download a file without an http client
            // "addpittie" => self.cmd_add_pittie(&msg, args).await,
            _ => {
                eprintln!("Unknown command: {}{}", CONFIG.prefix, args[0]);
                Ok(())
            }
        }
    }

    async fn cmd_pittie(self: Arc<Self>, msg: &Box<MessageCreate>) -> HandlerError {
        if let Some(attachment) = self.get_rand_attachment() {
            self.http
                .create_message(msg.channel_id)
                .attachments(&[attachment])?
                .exec()
                .await?;
        } else {
            self.http
                .create_message(msg.channel_id)
                .content("No images provided ):")?
                .exec()
                .await?;
        }
        Ok(())
    }

    // TODO
    // async fn handle_slash_cmd()

    async fn handle_event(self: Arc<Self>, event: Event) -> HandlerError {
        match event {
            Event::MessageCreate(msg)
                if !msg.author.bot && msg.content.starts_with(&CONFIG.prefix) =>
            {
                self.process_cmd(msg).await?;
            }
            Event::Ready(ready) => {
                println!(
                    "Logged in as {}#{:0>4}",
                    ready.user.name, ready.user.discriminator
                );
            }
            _ => {}
        }

        Ok(())
    }
}

// TODO clean shutdown
struct Pittie {
    _cluster: Cluster,
    events: Events,
    handler: Arc<EventHandler>,
}

impl Pittie {
    async fn init() -> anyhow::Result<Self> {
        let images = Self::get_images()?;

        // Initialise discord api related stuff
        let intents = Intents::GUILD_MESSAGES | Intents::MESSAGE_CONTENT;

        let (cluster, events) = Cluster::new(CONFIG.token.clone(), intents)
            .await
            .with_context(|| "failed to create cluster (is your token valid?)")?;

        // Start in the background until I need the cluster for the rest of the struct and use the join handle
        let handle = tokio::spawn(async move {
            cluster.up().await;
            cluster
        });

        let http = HttpClient::new(CONFIG.token.clone());

        let cluster = handle.await?;

        Ok(Self {
            _cluster: cluster,
            events,
            handler: EventHandler::new(http, images),
        })
    }

    fn get_images() -> anyhow::Result<Images> {
        let mut imagev = vec![];
        let path = PathBuf::from(IMAGES_DIR);

        let dir_iter = match fs::read_dir(&path) {
            Ok(ok) => ok,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&path).with_context(|| "Couldn't create image dir")?;
                fs::read_dir(&path).with_context(|| "Couldn't read image dir")?
            }
            Err(e) => anyhow::bail!("{}", e),
        };

        for f in dir_iter {
            // TODO I'm not entirely sure in which case this can return an error so look into this
            let f = f?;
            let filename = f.file_name().to_string_lossy().to_string();

            if supported_type(&filename) {
                let contents = read_file(path.join(&filename))?;
                imagev.push((filename, contents));
            }
        }

        Ok(RwLock::new(imagev))
    }

    async fn run(mut self) {
        while let Some((_, event)) = self.events.next().await {
            let handler = self.handler.clone();
            tokio::spawn(async move {
                if let Err(e) = handler.handle_event(event).await {
                    eprintln!("An error occurred while handling an event: {:?}", e);
                }
            });
        }
    }
}

// For a single server bot a single thread should be plenty
#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match Pittie::init().await {
        Ok(pittie) => {
            pittie.run().await;
            // TODO With my current setup this will never be reached unless my internet dies and the bot dies
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("Failed to initialize bot: {}", e);
            return ExitCode::FAILURE;
        }
    }
}

/// Simple helper which easily allows for adding more formats
#[inline]
fn supported_type(filename: &str) -> bool {
    filename.ends_with(".png") || filename.ends_with(".jpg") || filename.ends_with(".jpeg")
}

fn read_file(path: PathBuf) -> anyhow::Result<Vec<u8>> {
    let f = fs::File::open(path)?;
    let mut br = BufReader::new(f);

    let mut res = vec![];
    br.read_to_end(&mut res)?;

    Ok(res)
}

#[derive(Deserialize, Serialize)]
struct Config {
    admin_users: Vec<Id<UserMarker>>,
    prefix: String,
    token: String,
}

impl Config {
    fn init_config(config_path: &str) -> Self {
        let f = match fs::File::open(config_path) {
            Ok(ok) => ok,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                println!("No config found, creating a new one.");
                Self::create_default_config(config_path);
                std::process::exit(1);
            }
            Err(e) => panic!("Failed to open config file: {:?}", e),
        };

        match serde_json::from_reader(f) {
            Ok(ok) => ok,
            Err(_) => {
                eprintln!("Invalid config, resetting to default");
                Self::create_default_config(config_path);
                std::process::exit(1);
            }
        }
    }

    fn create_default_config(config_path: &str) {
        // Unwrap here is "safe", because it would fail anyway
        let file = fs::File::create(config_path).unwrap();

        let default = Config {
            admin_users: vec![],
            prefix: "%".into(),
            token: "your token here".into(),
        };

        // Unwrap here is also "safe", because it would fail anyway
        serde_json::to_writer_pretty(&file, &default).unwrap();
    }
}
