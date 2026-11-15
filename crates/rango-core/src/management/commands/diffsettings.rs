use async_trait::async_trait;
use clap::{Arg, ArgAction};

use crate::conf::{Settings, try_settings};
use crate::management::{Command, CommandContext, CommandError};

/// `diffsettings`: show settings that differ from Rango's defaults.
#[derive(Default)]
pub struct DiffSettingsCommand;

#[async_trait]
impl Command for DiffSettingsCommand {
    fn help(&self) -> &str {
        "Displays differences between the current settings and Rango's default settings."
    }

    fn requires_system_checks(&self) -> bool {
        false
    }

    fn add_arguments(&self, cmd: clap::Command) -> clap::Command {
        cmd.arg(
            Arg::new("all")
                .long("all")
                .action(ArgAction::SetTrue)
                .help("Display all settings, regardless of their value. In 'hash' mode, default values are prefixed by '###'."),
        )
    }

    async fn handle(
        &self,
        _: &CommandContext,
        args: &clap::ArgMatches,
    ) -> Result<Option<String>, CommandError> {
        let show_all = args.get_flag("all");
        let current = try_settings()?.to_map();
        let mut lines = Vec::new();
        for (name, value) in &current {
            let default = Settings::default_of(name);
            if default.is_none_or(|d| d != value) {
                lines.push(format!("{name} = {value}"));
            } else if show_all {
                lines.push(format!("### {name} = {value}"));
            }
        }
        Ok(Some(lines.join("\n")))
    }
}

crate::register_command!("rango.core", "diffsettings", DiffSettingsCommand);
