mod app;
mod audio;
mod config;
mod fx;
#[cfg(feature = "gui")]
mod gui;
mod index;
mod keys;
mod spectrum;
mod stderr;
mod tui;
mod ui;
use anyhow::Result;
use config::Ui;
const USAGE: &str = "usage: vinyl [--gui | --no-gui]
       vinyl --probe <audio file>

  --gui       open a desktop window instead of running in the terminal
  --no-gui    run in the terminal (the default unless `ui = gui` is set
              in ~/.vinyl)
  --probe     open a file the way playback does and describe it
  -h, --help  show this help
";
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--probe") {
        let config = config::load()?;
        let Some(path) = args.get(1) else {
            eprintln!("usage: vinyl --probe <audio file>");
            std::process::exit(2);
        };
        match audio::probe(std::path::Path::new(path), app::load_fx(&config)) {
            Ok(report) => print!("{report}"),
            Err(e) => {
                eprintln!("probe failed: {e:#}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    let mut ui = None;
    for arg in &args {
        match arg.as_str() {
            "--gui" => ui = Some(Ui::Gui),
            "--no-gui" | "--tui" => ui = Some(Ui::Tui),
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            other => {
                eprintln!("vinyl: unknown option `{other}`\n{USAGE}");
                std::process::exit(2);
            }
        }
    }
    let config = config::load()?;
    match ui.unwrap_or(config.ui) {
        Ui::Tui => tui::run(config),
        Ui::Gui => run_gui(config),
    }
}
#[cfg(feature = "gui")]
fn run_gui(config: config::Config) -> Result<()> {
    gui::run(config)
}
#[cfg(not(feature = "gui"))]
fn run_gui(_config: config::Config) -> Result<()> {
    Err(anyhow::anyhow!(
        "this vinyl was built without the desktop window; rebuild with \
         `cargo install --path . --features gui` or run without --gui"
    ))
}
