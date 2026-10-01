//! `edex-comp`: the eDEX Wayland compositor, and a small client for its control socket.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use comp_proto::{Rect, Request};
use edex_comp::state::InitOptions;

#[derive(Parser)]
#[command(name = "edex-comp", version, about = "The eDEX compositor for RustOS")]
#[command(args_conflicts_with_subcommands = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[command(flatten)]
    run: RunArgs,
}

#[derive(Args, Default)]
struct RunArgs {
    /// Run inside another Wayland or X11 session (development).
    #[arg(long)]
    nested: bool,
    /// Show the login screen; started as root by the RustOS service manager on tty1.
    #[arg(long)]
    greeter: bool,
    /// Start these programs instead of the eDEX session (repeatable).
    #[arg(long = "run", value_name = "COMMAND")]
    run: Vec<String>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the compositor (the default; `edex-comp --greeter` is `edex-comp run --greeter`).
    Run(RunArgs),
    /// Send a raw JSON request, e.g. `edex-comp msg '{"cmd":"state"}'`.
    Msg { json: String },
    /// Print windows, workspaces and outputs.
    State,
    /// Save a PNG of an output or of a region (`X,Y WxH`, as printed by slurp).
    Screenshot {
        #[arg(long)]
        output: Option<String>,
        #[arg(long)]
        region: Option<String>,
        path: PathBuf,
    },
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command.unwrap_or(Command::Run(cli.run)) {
        Command::Run(a) => run_compositor(a.nested, a.greeter, a.run),
        Command::Msg { json } => msg(&json),
        Command::State => state(),
        Command::Screenshot {
            output,
            region,
            path,
        } => screenshot(output, region, path),
    };
    if let Err(e) = result {
        eprintln!("edex-comp: {e:#}");
        std::process::exit(1);
    }
}

fn run_compositor(nested: bool, greeter: bool, run: Vec<String>) -> Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .compact()
        .with_env_filter(filter)
        .init();

    let runtime_dir = if greeter {
        // Before anyone logs in the compositor's sockets live in a root-only directory.
        let dir = PathBuf::from("/run/edex-comp");
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        dir
    } else {
        match std::env::var_os("XDG_RUNTIME_DIR") {
            Some(d) => PathBuf::from(d),
            None => {
                let user = edex_comp::session::current_user();
                edex_comp::session::ensure_runtime_dir(&user)?
            }
        }
    };
    // Smithay opens the Wayland socket in $XDG_RUNTIME_DIR.
    std::env::set_var("XDG_RUNTIME_DIR", &runtime_dir);
    let opts = InitOptions {
        greeter,
        config: settings::config_path(),
        runtime_dir,
        autostart: (!run.is_empty()).then_some(run),
    };
    if nested {
        tracing::info!("starting nested (winit)");
        edex_comp::winit::run_winit(opts);
    } else {
        tracing::info!("starting on this seat (DRM/KMS, libinput)");
        edex_comp::udev::run_udev(opts);
    }
    Ok(())
}

fn msg(json: &str) -> Result<()> {
    let request: Request = serde_json::from_str(json).context("not a valid request")?;
    let reply = comp_proto::call(&request)?;
    println!("{}", serde_json::to_string_pretty(&reply)?);
    Ok(())
}

fn state() -> Result<()> {
    let data = comp_proto::call(&Request::State)?;
    let snap: comp_proto::Snapshot = serde_json::from_value(data)?;
    for m in &snap.monitors {
        println!(
            "output {} {}x{}@{:.0} at {},{} scale {} workspace {}{}",
            m.name,
            m.width,
            m.height,
            m.refresh_rate,
            m.x,
            m.y,
            m.scale,
            m.active_workspace,
            if m.focused { " (focused)" } else { "" }
        );
    }
    for w in &snap.windows {
        println!(
            "window {} [{}] {:?} ws {}{}{} — {}",
            w.id,
            w.app_id,
            w.mode,
            w.workspace,
            if w.minimized { " minimized" } else { "" },
            if snap.focused == Some(w.id) {
                " focused"
            } else {
                ""
            },
            w.title
        );
    }
    println!("keyboard layout: {}", snap.keyboard_layout);
    Ok(())
}

fn screenshot(output: Option<String>, region: Option<String>, path: PathBuf) -> Result<()> {
    let region = match region {
        Some(r) => {
            let rect = edex_comp::control::parse_slurp(&r)
                .with_context(|| format!("bad region `{r}`, expected `X,Y WxH`"))?;
            Some(Rect::new(rect.loc.x, rect.loc.y, rect.size.w, rect.size.h))
        }
        None => None,
    };
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    };
    let Some(path) = path.to_str().map(str::to_string) else {
        bail!("the path is not UTF-8");
    };
    comp_proto::call(&Request::Screenshot {
        path,
        output,
        region,
    })?;
    Ok(())
}
