//! `rango-admin`: Rango's command-line utility (`rango-admin`).

#[tokio::main]
async fn main() {
    rango_cli::__link();
    // Commands that need no project (startproject, startapp, help, version) work with defaults.
    rango_core::conf::configure(rango_core::conf::Settings::default()).expect("fresh process");
    std::process::exit(rango_core::management::execute_from_command_line(std::env::args()).await);
}
