//! Command-line parsing. Pure logic — no OS calls — so it is fully
//! unit-tested on any platform.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Run {
        program: String,
        args: Vec<String>,
        /// `None` = use the configured default watch window.
        watch_secs: Option<u64>,
    },
    Hide {
        target: String,
    },
    List {
        json: bool,
    },
    Restore {
        target: String,
    },
    RestoreAll,
    Doctor,
    Export {
        path: String,
    },
    Import {
        path: String,
    },
    Tray,
    Help,
    Version,
}

pub fn parse(argv: &[String]) -> Result<Command, String> {
    let Some(first) = argv.first() else {
        // No arguments (e.g. double-clicked): start the tray.
        return Ok(Command::Tray);
    };
    match first.as_str() {
        "run" => parse_run(&argv[1..]),
        "tray" => Ok(Command::Tray),
        "hide" => parse_target(&argv[1..], "hide").map(|target| Command::Hide { target }),
        "restore" => parse_target(&argv[1..], "restore").map(|target| Command::Restore { target }),
        "restore-all" => Ok(Command::RestoreAll),
        "doctor" => Ok(Command::Doctor),
        "export" => parse_target(&argv[1..], "export").map(|path| Command::Export { path }),
        "import" => parse_target(&argv[1..], "import").map(|path| Command::Import { path }),
        "list" => Ok(Command::List {
            json: argv.get(1).map(|a| a == "--json").unwrap_or(false),
        }),
        "help" | "--help" | "-h" | "/?" => Ok(Command::Help),
        "version" | "--version" | "-V" => Ok(Command::Version),
        // Shorthand: `hushrun notepad.exe` means `hushrun run notepad.exe`.
        _ => parse_run(argv),
    }
}

fn parse_run(rest: &[String]) -> Result<Command, String> {
    let mut watch_secs = None;
    let mut idx = 0;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--watch" => {
                let value = rest
                    .get(idx + 1)
                    .ok_or("--watch needs a value in seconds, e.g. --watch 30")?;
                watch_secs = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| format!("--watch value '{value}' is not a whole number"))?,
                );
                idx += 2;
            }
            "--" => {
                idx += 1;
                break;
            }
            _ => break,
        }
    }
    let program = rest
        .get(idx)
        .ok_or("run needs a program, e.g. hushrun run notepad.exe")?
        .clone();
    let args = rest[idx + 1..].to_vec();
    Ok(Command::Run {
        program,
        args,
        watch_secs,
    })
}

fn parse_target(rest: &[String], verb: &str) -> Result<String, String> {
    match rest {
        [target] => Ok(target.clone()),
        [] => Err(format!(
            "{verb} needs a process id or program name, e.g. hushrun {verb} notepad"
        )),
        _ => Err(format!("{verb} takes exactly one target")),
    }
}

pub const HELP: &str = "\
HushRun — run programs silently in the background

USAGE:
  hushrun                                start the tray icon (same as `tray`)
  hushrun tray                           start the tray icon
  hushrun run [--watch <seconds>] <program> [args...]
  hushrun <program> [args...]            (shorthand for run)
  hushrun hide <pid|name|pattern>        hide a running program's windows
                                         (pattern may contain *, e.g. 'note*')
  hushrun list [--json]                  list hidden sessions
  hushrun restore <pid|name|pattern>     bring a hidden program back
  hushrun restore-all                    bring everything back
  hushrun doctor                         check the store, config and sessions
  hushrun export <file>                  write settings to a file
  hushrun import <file>                  load settings from a file
  hushrun version

`run` starts the program with its window hidden and keeps hiding
windows it opens for --watch seconds (default: the configured
watch window, 15 unless changed in the tray's Settings).
Sessions are remembered, so `restore` works from a later
invocation too. The tray icon watches hidden programs
continuously and restores them from its menu.
";

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn run_with_watch_and_args() {
        let cmd = parse(&v(&["run", "--watch", "30", "notepad.exe", "a.txt"])).unwrap();
        assert_eq!(
            cmd,
            Command::Run {
                program: "notepad.exe".into(),
                args: v(&["a.txt"]),
                watch_secs: Some(30)
            }
        );
    }

    #[test]
    fn shorthand_run_uses_configured_watch() {
        let cmd = parse(&v(&["calc.exe"])).unwrap();
        assert_eq!(
            cmd,
            Command::Run {
                program: "calc.exe".into(),
                args: vec![],
                watch_secs: None
            }
        );
    }

    #[test]
    fn program_args_that_look_like_flags_pass_through() {
        let cmd = parse(&v(&["run", "tool.exe", "--watch", "not-ours"])).unwrap();
        match cmd {
            Command::Run { args, .. } => assert_eq!(args, v(&["--watch", "not-ours"])),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn bad_watch_value_is_an_error() {
        assert!(parse(&v(&["run", "--watch", "soon", "x.exe"])).is_err());
        assert!(parse(&v(&["run", "--watch"])).is_err());
    }

    #[test]
    fn targets() {
        assert_eq!(
            parse(&v(&["hide", "1234"])).unwrap(),
            Command::Hide {
                target: "1234".into()
            }
        );
        assert!(parse(&v(&["hide"])).is_err());
        assert!(parse(&v(&["restore", "a", "b"])).is_err());
        assert_eq!(parse(&v(&["restore-all"])).unwrap(), Command::RestoreAll);
        assert_eq!(parse(&v(&["list"])).unwrap(), Command::List { json: false });
        assert_eq!(
            parse(&v(&["list", "--json"])).unwrap(),
            Command::List { json: true }
        );
        assert_eq!(parse(&v(&["doctor"])).unwrap(), Command::Doctor);
        assert_eq!(
            parse(&v(&["export", "out.json"])).unwrap(),
            Command::Export {
                path: "out.json".into()
            }
        );
    }

    #[test]
    fn empty_is_tray_and_help_is_explicit() {
        assert_eq!(parse(&[]).unwrap(), Command::Tray);
        assert_eq!(parse(&v(&["tray"])).unwrap(), Command::Tray);
        assert_eq!(parse(&v(&["help"])).unwrap(), Command::Help);
    }
}
