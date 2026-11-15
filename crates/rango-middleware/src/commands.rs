//! `runserver`.

use std::sync::Arc;

use async_trait::async_trait;
use clap::Arg;
use rango_core::conf::settings;
use rango_core::management::{CheckOptions, Command, CommandContext, CommandError, run_system_checks};
use regex::Regex;
use tokio::net::TcpListener;

use crate::handlers::Handler;
use crate::servers::serve;

#[derive(Default)]
pub struct RunServerCommand;

/// `[addr:]port` as accepted by `runserver`.
pub fn parse_addrport(addrport: &str, ipv6: bool) -> Result<(String, u16), CommandError> {
    static RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r"^(?:(?P<addr>(?P<ipv4>\d{1,3}(?:\.\d{1,3}){3})|(?P<ipv6>\[[a-fA-F0-9:]+\])|(?P<fqdn>[a-zA-Z0-9-]+(?:\.[a-zA-Z0-9-]+)*)):)?(?P<port>\d+)$").expect("valid regex")
    });
    let default = if ipv6 { "::1" } else { "127.0.0.1" };
    if addrport.is_empty() {
        return Ok((default.into(), 8000));
    }
    let m = RE.captures(addrport).ok_or_else(|| CommandError::new(format!("\"{addrport}\" is not a valid port number or address:port pair.")))?;
    let addr = m.name("addr").map_or(default, |a| a.as_str()).trim_matches(['[', ']']).to_owned();
    Ok((if addr == "0" { "0.0.0.0".into() } else { addr }, m["port"].parse().map_err(|_| CommandError::new("invalid port"))?))
}

#[async_trait]
impl Command for RunServerCommand {
    fn help(&self) -> &str {
        "Starts a lightweight web server for development."
    }

    fn requires_system_checks(&self) -> bool {
        false
    }

    fn add_arguments(&self, cmd: clap::Command) -> clap::Command {
        cmd.arg(Arg::new("addrport").help("Optional port number, or ipaddr:port"))
            .arg(Arg::new("ipv6").short('6').long("ipv6").action(clap::ArgAction::SetTrue).help("Tells Rango to use an IPv6 address."))
            .arg(Arg::new("nothreading").long("nothreading").action(clap::ArgAction::SetTrue).help("Accepted for compatibility; the server is always async."))
            .arg(Arg::new("noreload").long("noreload").action(clap::ArgAction::SetTrue).help("Accepted for compatibility; use `cargo watch -x run` for reloading."))
            .arg(Arg::new("skip_checks").long("skip-checks").action(clap::ArgAction::SetTrue).help("Skip system checks."))
    }

    async fn handle(&self, ctx: &CommandContext, args: &clap::ArgMatches) -> Result<Option<String>, CommandError> {
        let (addr, port) = parse_addrport(args.get_one::<String>("addrport").map_or("", String::as_str), args.get_flag("ipv6"))?;
        if !settings().debug && settings().allowed_hosts.is_empty() {
            return Err(CommandError::new("You must set settings.ALLOWED_HOSTS if DEBUG is False."));
        }
        if !args.get_flag("skip_checks") {
            ctx.stdout.write("Performing system checks...\n");
            run_system_checks(ctx, &CheckOptions { display_num_errors: true, ..CheckOptions::default() }).await?;
        }
        let handler = Arc::new(Handler::new()?);
        let listener = TcpListener::bind((addr.as_str(), port)).await.map_err(|e| CommandError::new(format!("Error: {e}")))?;
        let shown = if addr.contains(':') { format!("[{addr}]") } else { addr.clone() };
        let now = chrono::Local::now().format("%B %d, %Y - %H:%M:%S");
        ctx.stdout.write(&format!(
            "{now}\nRango version {}\nStarting development server at http://{shown}:{port}/\nQuit the server with CONTROL-C.",
            rango_core::management::VERSION
        ));
        serve(handler, listener, true).await.map_err(|e| CommandError::new(e.to_string()))?;
        Ok(None)
    }
}

rango_core::register_command!("rango.core", "runserver", RunServerCommand);
