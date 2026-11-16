//! S1-style smoke test: settings + app registry + checks + management commands + signals,
//! exercised the way a project's `cargo run -- <command>` does.

use rango::apps::AppConfig;
use rango::conf::{self, Settings, override_settings};
use rango::core::checks::{self, CheckContext, CheckMessage};
use rango::core::management::{
    Command, CommandContext, CommandError, Output, Sink, call_command, execute, get_commands,
    main_help_text,
};
use rango::dispatch::{ConnectOptions, Signal};
use rango::{register_app, register_check, register_command};

#[derive(Default)]
struct Echo;

#[async_trait::async_trait]
impl Command for Echo {
    fn help(&self) -> &str {
        "Echo arguments."
    }
    fn add_arguments(&self, cmd: clap::Command) -> clap::Command {
        cmd.arg(clap::Arg::new("text").required(true))
    }
    async fn handle(
        &self,
        ctx: &CommandContext,
        args: &clap::ArgMatches,
    ) -> Result<Option<String>, CommandError> {
        if ctx.verbosity >= 2 {
            ctx.stdout.write("(verbose)");
        }
        Ok(Some(
            args.get_one::<String>("text").cloned().unwrap_or_default(),
        ))
    }
}

#[derive(Default)]
struct Hidden;

#[async_trait::async_trait]
impl Command for Hidden {
    async fn handle(
        &self,
        _: &CommandContext,
        _: &clap::ArgMatches,
    ) -> Result<Option<String>, CommandError> {
        Ok(None)
    }
}

static READY_RAN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

register_app!(
    AppConfig::new("polls")
        .verbose_name("Polls")
        .ready(|| Box::pin(async {
            READY_RAN.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }))
);
register_app!(AppConfig::new("unused"));
register_command!("polls", "echo", Echo);
register_command!("unused", "hidden", Hidden);

/// Like the `security.W020`: ALLOWED_HOSTS must not be empty when DEBUG is off.
async fn allowed_hosts_check(_: &CheckContext) -> Vec<CheckMessage> {
    let s = conf::settings();
    if !s.debug && s.allowed_hosts.is_empty() {
        vec![
            CheckMessage::error("ALLOWED_HOSTS must not be empty in production.")
                .with_id("demo.E001"),
        ]
    } else {
        vec![]
    }
}
register_check!(tags = [checks::tags::SECURITY], allowed_hosts_check);

static GREETED: Signal<String, String> = Signal::new();

async fn run(argv: &[&str]) -> (Result<(), CommandError>, String, String) {
    let (out, err) = (
        std::sync::Arc::new(std::sync::Mutex::new(String::new())),
        std::sync::Arc::new(std::sync::Mutex::new(String::new())),
    );
    let args: Vec<String> = argv[1..].iter().map(|s| (*s).to_owned()).collect();
    let r = execute(
        argv[0],
        &args,
        Output::new(Sink::Buffer(out.clone())),
        Output::new(Sink::Buffer(err.clone())),
    )
    .await;
    let (o, e) = (out.lock().unwrap().clone(), err.lock().unwrap().clone());
    (r, o, e)
}

#[tokio::test]
async fn manage_py_flow() {
    conf::configure(Settings {
        debug: true,
        secret_key: "k".into(),
        installed_apps: vec!["polls".into()],
        ..Settings::default()
    })
    .unwrap();
    assert!(
        conf::configure(Settings::default()).is_err(),
        "double configure must fail"
    );

    // App registry: only installed apps, ready() hooks ran.
    rango::setup().await.unwrap();
    assert!(READY_RAN.load(std::sync::atomic::Ordering::SeqCst));
    let apps = rango::apps::apps().unwrap();
    assert_eq!(apps.get_app_config("polls").unwrap().verbose_name, "Polls");
    assert!(apps.get_app_config("nope").is_err());

    // Command discovery: core commands always, app commands only when installed.
    let cmds = get_commands();
    assert_eq!(cmds.get("check"), Some(&"rango.core"));
    assert_eq!(cmds.get("echo"), Some(&"polls"));
    assert!(!cmds.contains_key("hidden"));
    let help = main_help_text("manage");
    assert!(
        help.contains("[polls]\n    echo\n") && help.contains("[rango]\n"),
        "{help}"
    );

    // call_command + app command + verbosity + usage errors.
    assert_eq!(call_command("echo", &["hi"]).await.unwrap().trim(), "hi");
    assert!(
        call_command("echo", &["-v", "2", "hi"])
            .await
            .unwrap()
            .starts_with("(verbose)")
    );
    assert_eq!(run(&["echo"]).await.0.unwrap_err().returncode, 2);
    assert!(call_command("hidden", &[]).await.is_err());

    // `version`, `diffsettings`.
    assert_eq!(
        call_command("version", &[]).await.unwrap().trim(),
        rango::VERSION
    );
    let diff = call_command("diffsettings", &[]).await.unwrap();
    assert!(
        diff.contains("DEBUG = true")
            && diff.contains("INSTALLED_APPS = [\"polls\"]")
            && !diff.contains("APPEND_SLASH"),
        "{diff}"
    );
    assert!(
        call_command("diffsettings", &["--all"])
            .await
            .unwrap()
            .contains("### APPEND_SLASH = true")
    );

    // System checks: clean, then failing under override_settings, then silenced.
    let (r, out, _) = run(&["check"]).await;
    assert!(r.is_ok());
    assert_eq!(
        out.trim(),
        "System check identified no issues (0 silenced)."
    );
    override_settings(|s| s.debug = false, async {
        let (r, _, _) = run(&["check"]).await;
        let e = r.unwrap_err();
        assert!(
            e.is_system_check
                && e.message.contains("demo.E001")
                && e.message.contains("1 issue (0 silenced)"),
            "{}",
            e.message
        );
        // Commands that require checks fail early; --skip-checks bypasses.
        assert!(run(&["echo", "x"]).await.0.is_err());
        assert!(run(&["echo", "--skip-checks", "x"]).await.0.is_ok());
        // Not failing at WARNING level when the fail level is higher than the issue.
        assert!(run(&["check", "--fail-level", "CRITICAL"]).await.0.is_ok());
        // Silencing by id.
        override_settings(
            |s| s.silenced_system_checks = vec!["demo.E001".into()],
            async {
                let (r, out, _) = run(&["check"]).await;
                assert!(r.is_ok());
                assert!(out.contains("no issues (1 silenced)"), "{out}");
            },
        )
        .await;
    })
    .await;
    // Tag filter and --list-tags.
    assert_eq!(
        call_command("check", &["--list-tags"])
            .await
            .unwrap()
            .trim(),
        "security"
    );

    // Signals: ordering, sender filter, dispatch_uid de-duplication, send_robust panics.
    GREETED.connect_sync(|d| format!("a:{}", d.args), ConnectOptions::default());
    GREETED.connect_sync(
        |d| format!("b:{}", d.args),
        ConnectOptions::sender("polls.Question").uid("once"),
    );
    GREETED.connect_sync(
        |_| String::from("dupe"),
        ConnectOptions::sender("polls.Question").uid("once"),
    );
    GREETED.connect_sync(
        |_| -> String { panic!("boom") },
        ConnectOptions::sender("polls.Boom"),
    );
    let all: Vec<_> = GREETED
        .send(Some("polls.Question"), "x".into())
        .await
        .into_iter()
        .map(|(_, r)| r)
        .collect();
    assert_eq!(all, ["a:x", "b:x"]);
    let anon: Vec<_> = GREETED
        .send(None, "y".into())
        .await
        .into_iter()
        .map(|(_, r)| r)
        .collect();
    assert_eq!(anon, ["a:y"]);
    let robust = GREETED.send_robust(Some("polls.Boom"), "z".into()).await;
    assert_eq!(robust.len(), 2);
    assert_eq!(robust[1].1, Err("boom".to_owned()));
}
