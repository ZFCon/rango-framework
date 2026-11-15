//! `createsuperuser` and `changepassword`.

use async_trait::async_trait;
use clap::{Arg, ArgAction};
use rango_core::management::{Command, CommandContext, CommandError};
use rango_db::Db;

use super::get_user_model;

fn env(name: &str) -> Option<String> {
    std::env::var(format!("RANGO_SUPERUSER_{name}")).ok()
}

/// `createsuperuser --noinput --username U --email E` (password from `RANGO_SUPERUSER_PASSWORD`).
#[derive(Default)]
pub struct CreateSuperuser;

#[async_trait]
impl Command for CreateSuperuser {
    fn help(&self) -> &str {
        "Used to create a superuser."
    }
    fn add_arguments(&self, cmd: clap::Command) -> clap::Command {
        cmd.arg(Arg::new("username").long("username"))
            .arg(Arg::new("email").long("email"))
            .arg(Arg::new("noinput").long("noinput").visible_alias("no-input").action(ArgAction::SetTrue))
            .arg(Arg::new("database").long("database").default_value("default"))
    }
    async fn handle(&self, ctx: &CommandContext, args: &clap::ArgMatches) -> Result<Option<String>, CommandError> {
        let um = get_user_model().map_err(|e| CommandError::new(e.to_string()))?;
        let username = args.get_one::<String>("username").cloned().or_else(|| env("USERNAME")).ok_or_else(|| CommandError::new("You must use --username with --noinput."))?;
        let email = args.get_one::<String>("email").cloned().or_else(|| env("EMAIL")).unwrap_or_default();
        let password = env("PASSWORD");
        let db = Db::using(args.get_one::<String>("database").map_or("default", String::as_str));
        if um.get_by_natural_key(&db, &username).await.map_err(|e| CommandError::new(e.to_string()))?.is_some() {
            return Err(CommandError::new("Error: That username is already taken."));
        }
        um.create_superuser(&db, &username, &email, password.as_deref()).await.map_err(|e| CommandError::new(e.to_string()))?;
        ctx.stdout.write("Superuser created successfully.");
        Ok(None)
    }
}

rango_core::register_command!("rango.contrib.auth", "createsuperuser", CreateSuperuser);

/// `changepassword <username> --password P`.
#[derive(Default)]
pub struct ChangePassword;

#[async_trait]
impl Command for ChangePassword {
    fn help(&self) -> &str {
        "Change a user's password for rango.contrib.auth."
    }
    fn add_arguments(&self, cmd: clap::Command) -> clap::Command {
        cmd.arg(Arg::new("username").required(true)).arg(Arg::new("password").long("password").required(true)).arg(Arg::new("database").long("database").default_value("default"))
    }
    async fn handle(&self, ctx: &CommandContext, args: &clap::ArgMatches) -> Result<Option<String>, CommandError> {
        let um = get_user_model().map_err(|e| CommandError::new(e.to_string()))?;
        let username = args.get_one::<String>("username").expect("required");
        let db = Db::using(args.get_one::<String>("database").map_or("default", String::as_str));
        let mut user = um.get_by_natural_key(&db, username).await.map_err(|e| CommandError::new(e.to_string()))?.ok_or_else(|| CommandError::new(format!("user '{username}' does not exist")))?;
        user.set_password(args.get_one::<String>("password").expect("required")).map_err(|e| CommandError::new(e.to_string()))?;
        user.save_fields(&db, &["password"]).await.map_err(|e| CommandError::new(e.to_string()))?;
        ctx.stdout.write(&format!("Password changed successfully for user '{username}'"));
        Ok(None)
    }
}

rango_core::register_command!("rango.contrib.auth", "changepassword", ChangePassword);
