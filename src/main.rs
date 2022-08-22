use anyhow::Context as _;
use serenity::{
    model::prelude::{AttachmentType, Message, Ready, UserId},
    prelude::{Context, EventHandler, GatewayIntents},
    Client,
};
use std::sync::RwLock;
use std::{fs, io::ErrorKind, path::PathBuf};

const IMAGES_PATH: &str = "./images";
const CONFIG_PATH: &str = "./pittie_config.json";

#[derive(serde::Deserialize, serde::Serialize)]
struct Config {
    token: String,
    prefix: String,
    admins: Vec<UserId>,
}

impl Config {
    fn init(path: &str) -> anyhow::Result<Option<Self>> {
        let config: Self = match fs::File::open(path) {
            Ok(ok) => serde_json::from_reader(ok).with_context(|| "Parsing config failed")?,
            Err(e) if ErrorKind::NotFound == e.kind() => {
                let default_config = Config {
                    token: "Insert your token here".into(),
                    prefix: "%".into(),
                    admins: vec![],
                };

                let f = fs::File::create(path)?;

                serde_json::to_writer_pretty(f, &default_config)
                    .with_context(|| "Failed to write default configuration")?;

                return Ok(None);
            }
            Err(e) => Err(e)?,
        };

        Ok(Some(config))
    }

    // fn is_admin(&self, id: &UserId) -> bool {
    //     self.admins.contains(id)
    // }
}

struct Pittie2 {
    config: Config,
    img_paths: RwLock<Vec<PathBuf>>,
}

impl Pittie2 {
    // Returns Ok(None) if the bot hasn't been started yet and the config and image dirs are just created
    pub fn new() -> anyhow::Result<Option<Self>> {
        if let Some(config) = Config::init(CONFIG_PATH)? {
            let mut img_paths = vec![];

            match fs::read_dir(IMAGES_PATH) {
                Ok(ok) => {
                    for file in ok {
                        if let Ok(file) = file {
                            if is_img(&file.file_name().to_string_lossy()) {
                                img_paths.push(file.path());
                            }
                        }
                    }
                }
                Err(e) if e.kind() == ErrorKind::NotFound => {
                    fs::create_dir(IMAGES_PATH)?;
                }
                Err(e) => Err(e)?,
            }

            Ok(Some(Self {
                config,
                img_paths: RwLock::new(img_paths),
            }))
        } else {
            Ok(None)
        }
    }

    fn prefix<'a>(&'a self) -> &'a String {
        &self.config.prefix
    }

    async fn run(self) {
        let intents = GatewayIntents::GUILD_MESSAGES | GatewayIntents::MESSAGE_CONTENT;

        let mut client = Client::builder(&self.config.token, intents)
            .event_handler(self)
            .await
            .expect("Failed to create client");

        if let Err(e) = client.start().await {
            println!("Client error: {:?}", e);
        }
    }

    fn get_rand_path(&self) -> Option<PathBuf> {
        Some({
            let read = self
                .img_paths
                .read()
                .expect("Failed to acquire img_paths read lock");

            if read.is_empty() {
                return None;
            }

            read[fastrand::usize(..read.len())].clone()
        })
    }
}

#[serenity::async_trait]
impl EventHandler for Pittie2 {
    async fn message(&self, ctx: Context, msg: Message) {
        let args: Vec<&str> = msg.content.split(" ").collect();
        let prefix = self.prefix();

        if args[0].starts_with(prefix) {
            let name = &args[0][prefix.len()..];

            match name {
                "pittie" => {
                    // I don't think I need to care if this errors
                    let _typing = msg.channel_id.start_typing(&ctx.http);
                    let rand_path = self.get_rand_path();

                    if let Err(e) = msg
                        .channel_id
                        .send_message(&ctx.http, |msg| {
                            if let Some(ref path) = rand_path {
                                msg.add_file(AttachmentType::Path(path))
                            } else {
                                msg.content("No images provided ):")
                            }
                        })
                        .await
                    {
                        eprintln!("Failed to run command: {:?}", e);
                    }
                }
                // TODO add this later
                "addpittie" => {}
                _ => eprintln!("Unknown command: {}", name),
            }
        }
    }

    async fn ready(&self, _ctx: Context, ready: Ready) {
        println!("Logged in as {}", ready.user.tag());
    }
}

// TODO perhaps consider switching to serenitys built-in command framework instead of the current

#[tokio::main(flavor = "current_thread")]
async fn main() {
    match Pittie2::new() {
        Ok(ok) => {
            if let Some(pittie2) = ok {
                {
                    // Unwrap here is safe because no writer has been created yet
                    let images = pittie2.img_paths.read().unwrap();
                    if images.is_empty() {
                        eprintln!("No images found in: {}", IMAGES_PATH);
                    }
                }

                pittie2.run().await;
            } else {
                println!("Created default config");
            }
        }
        Err(e) => eprintln!("Error while initializing: {}", e),
    }
}

fn is_img(s: &str) -> bool {
    s.ends_with(".png") || s.ends_with(".jpg") || s.ends_with(".jpeg")
}
