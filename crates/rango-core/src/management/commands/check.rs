use async_trait::async_trait;
use clap::{Arg, ArgAction};

use crate::checks::{self, CheckContext};
use crate::management::{CheckOptions, Command, CommandContext, CommandError, run_system_checks};

/// `check`: inspect the project for common problems.
#[derive(Default)]
pub struct CheckCommand;

#[async_trait]
impl Command for CheckCommand {
    fn help(&self) -> &str {
        "Checks the entire Rango project for potential problems."
    }

    fn requires_system_checks(&self) -> bool {
        false
    }

    fn add_arguments(&self, cmd: clap::Command) -> clap::Command {
        cmd.arg(Arg::new("app_label").num_args(0..))
            .arg(Arg::new("tag").short('t').long("tag").action(ArgAction::Append).help("Run only checks labeled with given tag."))
            .arg(Arg::new("list_tags").long("list-tags").action(ArgAction::SetTrue).help("List available tags."))
            .arg(Arg::new("deploy").long("deploy").action(ArgAction::SetTrue).help("Check deployment settings."))
            .arg(
                Arg::new("fail_level")
                    .long("fail-level")
                    .value_parser(["CRITICAL", "ERROR", "WARNING", "INFO", "DEBUG"])
                    .default_value("ERROR")
                    .help("Message level that will cause the command to exit with a non-zero status. Default is ERROR."),
            )
            .arg(Arg::new("database").long("database").action(ArgAction::Append).help("Run database related checks against these aliases."))
    }

    async fn handle(
        &self,
        ctx: &CommandContext,
        args: &clap::ArgMatches,
    ) -> Result<Option<String>, CommandError> {
        let deploy = args.get_flag("deploy");
        if args.get_flag("list_tags") {
            return Ok(Some(checks::tags_available(deploy).join("\n")));
        }
        let many = |id: &str| -> Option<Vec<String>> {
            args.get_many::<String>(id)
                .map(|v| v.cloned().collect::<Vec<_>>())
                .filter(|v| !v.is_empty())
        };
        let fail_level = match args.get_one::<String>("fail_level").map(String::as_str) {
            Some("CRITICAL") => checks::CRITICAL,
            Some("WARNING") => checks::WARNING,
            Some("INFO") => checks::INFO,
            Some("DEBUG") => checks::DEBUG,
            _ => checks::ERROR,
        };
        run_system_checks(
            ctx,
            &CheckOptions {
                context: CheckContext {
                    app_labels: many("app_label"),
                    databases: many("database"),
                },
                tags: many("tag"),
                display_num_errors: true,
                include_deployment_checks: deploy,
                fail_level,
            },
        )
        .await?;
        Ok(None)
    }
}

crate::register_command!("rango.core", "check", CheckCommand);
