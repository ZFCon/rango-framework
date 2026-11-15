use async_trait::async_trait;

use crate::management::{Command, CommandContext, CommandError, VERSION};

/// `version`: print the Rango version.
#[derive(Default)]
pub struct VersionCommand;

#[async_trait]
impl Command for VersionCommand {
    fn help(&self) -> &str {
        "Display the Rango version."
    }

    fn requires_system_checks(&self) -> bool {
        false
    }

    async fn handle(
        &self,
        _: &CommandContext,
        _: &clap::ArgMatches,
    ) -> Result<Option<String>, CommandError> {
        Ok(Some(VERSION.to_owned()))
    }
}

crate::register_command!("rango.core", "version", VersionCommand);
