use anyhow::{Result, bail};
use herdrpp::{
    picker::{api::Herdr, cli},
    pr::watcher::{self, Session},
};

fn execute(action: &str) -> Result<()> {
    match action {
        "open" | "pick" | "check" => cli::run(&Herdr::default(), action),
        "start" => Session::load()?.start(),
        "watch" => Session::load()?.watch(),
        "refresh" => Session::load()?.refresh(true),
        "stop" => Session::load()?.stop(),
        "check-pr" => watcher::check(),
        "configure" => herdrpp::configure::run(),
        "--version" | "-V" => {
            println!("herdrpp {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "--help" | "-h" => {
            println!(
                "herdr++ {}\n\nopen       Search workspaces, worktrees and tabs\ncheck      Validate workspace discovery without changing focus\nstart      Start one background PR watcher per Herdr session\nrefresh    Refresh workspace PR metadata once\nstop       Request watcher shutdown\ncheck-pr   Read PR associations without reporting metadata\nconfigure  Apply pending viewing config setup once\npick       Internal popup entrypoint\nwatch      Internal watcher entrypoint\n\nPR commands require git and authenticated gh; picker commands do not.\nconfigure honors HERDR_CONFIG_PATH and records setup outside config.",
                env!("CARGO_PKG_VERSION")
            );
            Ok(())
        }
        _ => bail!("unknown command {action}; use --help"),
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let action = args.next().unwrap_or_else(|| "--help".into());
    let valid_arguments = args.next().is_none();
    let result = if !valid_arguments {
        Err(anyhow::anyhow!("expected a single command; use --help"))
    } else {
        execute(&action)
    };
    if let Err(error) = result {
        let message = format!("{error:#}");
        eprintln!("herdrpp: {message}");
        if valid_arguments
            && matches!(action.as_str(), "open" | "pick")
            && let Err(error) = Herdr::default().notify(&message)
        {
            eprintln!("herdrpp: could not show error notification: {error:#}");
        }
        std::process::exit(1);
    }
}
