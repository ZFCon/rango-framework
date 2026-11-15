//! `manage.py` for a tiny demo project: `cargo run -p rango --example manage -- help`.

use rango::apps::AppConfig;
use rango::conf::{self, Settings};
use rango::core::management::{Command, CommandContext, CommandError, execute_from_command_line};
use rango::{register_app, register_command};

#[derive(Default)]
struct Greet;

#[async_trait::async_trait]
impl Command for Greet {
    fn help(&self) -> &str {
        "Say hello."
    }

    fn add_arguments(&self, cmd: clap::Command) -> clap::Command {
        cmd.arg(clap::Arg::new("name").default_value("world"))
    }

    async fn handle(
        &self,
        _: &CommandContext,
        args: &clap::ArgMatches,
    ) -> Result<Option<String>, CommandError> {
        Ok(Some(format!(
            "Hello, {}!",
            args.get_one::<String>("name").expect("has default")
        )))
    }
}

register_app!(AppConfig::new("demo").verbose_name("Demo"));
register_command!("demo", "greet", Greet);

#[tokio::main]
async fn main() {
    conf::configure(Settings {
        debug: true,
        secret_key: "insecure-demo-key".into(),
        installed_apps: vec!["demo".into()],
        ..Settings::default()
    })
    .expect("configure settings");
    rango::setup().await.expect("setup");
    std::process::exit(execute_from_command_line(std::env::args()).await);
}
