//! Resolves the config directory of CLI tools that may live on Windows or in WSL.
//!
//! Order: manual override (Settings) -> environment variable -> `%USERPROFILE%` -> WSL.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};

use super::cmd::{decode_console_output, run_hidden};

const WSL_CACHE_TTL: Duration = Duration::from_secs(600);
const WSL_LIST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tool {
    Claude,
    Codex,
}

impl Tool {
    pub fn env_var(self) -> &'static str {
        match self {
            Tool::Claude => "CLAUDE_CONFIG_DIR",
            Tool::Codex => "CODEX_HOME",
        }
    }

    pub fn dir_name(self) -> &'static str {
        match self {
            Tool::Claude => ".claude",
            Tool::Codex => ".codex",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirSource {
    Manual,
    Env,
    Native,
    Wsl { distro: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDir {
    pub path: PathBuf,
    pub source: DirSource,
}

impl ResolvedDir {
    /// Processes inside WSL are invisible to the Windows process list.
    pub fn is_wsl(&self) -> bool {
        matches!(self.source, DirSource::Wsl { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WslHome {
    pub distro: String,
    pub home: PathBuf,
}

/// Inputs of the resolution, injectable for tests.
pub struct Lookup<'a> {
    pub manual: Option<PathBuf>,
    pub env: Option<PathBuf>,
    pub home: Option<PathBuf>,
    /// Only called when nothing was found on the Windows side.
    pub wsl_homes: &'a dyn Fn() -> Vec<WslHome>,
    pub exists: &'a dyn Fn(&Path) -> bool,
}

pub fn resolve_dir(tool: Tool, lookup: &Lookup<'_>) -> Option<ResolvedDir> {
    let exists = lookup.exists;

    if let Some(path) = lookup.manual.as_ref().filter(|path| exists(path)) {
        return Some(ResolvedDir {
            path: path.clone(),
            source: DirSource::Manual,
        });
    }
    if let Some(path) = lookup.env.as_ref().filter(|path| exists(path)) {
        return Some(ResolvedDir {
            path: path.clone(),
            source: DirSource::Env,
        });
    }
    if let Some(home) = &lookup.home {
        let path = home.join(tool.dir_name());
        if exists(&path) {
            return Some(ResolvedDir {
                path,
                source: DirSource::Native,
            });
        }
    }
    (lookup.wsl_homes)().into_iter().find_map(|wsl| {
        let path = wsl.home.join(tool.dir_name());
        exists(&path).then_some(ResolvedDir {
            path,
            source: DirSource::Wsl { distro: wsl.distro },
        })
    })
}

/// Parses the output of `wsl.exe -l -q` into distro names.
pub fn parse_wsl_distros(output: &str) -> Vec<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        // Docker's internal distros never hold user tools.
        .filter(|line| !line.to_ascii_lowercase().starts_with("docker-desktop"))
        .map(str::to_string)
        .collect()
}

fn discover_wsl_homes() -> Vec<WslHome> {
    let Some(output) = run_hidden("wsl.exe", &["-l", "-q"], WSL_LIST_TIMEOUT) else {
        return Vec::new();
    };
    let mut homes = Vec::new();
    for distro in parse_wsl_distros(&decode_console_output(&output)) {
        let root = PathBuf::from(format!(r"\\wsl.localhost\{distro}"));
        if let Ok(entries) = std::fs::read_dir(root.join("home")) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    homes.push(WslHome {
                        distro: distro.clone(),
                        home: entry.path(),
                    });
                }
            }
        }
        homes.push(WslHome {
            distro: distro.clone(),
            home: root.join("root"),
        });
    }
    homes
}

/// Thread-safe resolver with a cached WSL probe. All methods may block on the
/// filesystem, so call them from a blocking context.
#[derive(Default)]
pub struct PathResolver {
    manual: RwLock<HashMap<Tool, PathBuf>>,
    wsl_cache: Mutex<Option<(Instant, Vec<WslHome>)>>,
}

impl PathResolver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_manual(&self, tool: Tool, path: Option<PathBuf>) {
        let mut manual = self.manual.write().unwrap_or_else(|e| e.into_inner());
        match path {
            Some(path) => {
                manual.insert(tool, path);
            }
            None => {
                manual.remove(&tool);
            }
        }
    }

    pub fn resolve(&self, tool: Tool) -> Option<ResolvedDir> {
        let manual = self
            .manual
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&tool)
            .cloned();
        let lookup = Lookup {
            manual,
            env: std::env::var_os(tool.env_var())
                .filter(|value| !value.is_empty())
                .map(PathBuf::from),
            home: std::env::var_os("USERPROFILE")
                .or_else(|| std::env::var_os("HOME"))
                .map(PathBuf::from),
            wsl_homes: &|| self.wsl_homes(),
            exists: &|path| path.is_dir(),
        };
        resolve_dir(tool, &lookup)
    }

    fn wsl_homes(&self) -> Vec<WslHome> {
        let mut cache = self.wsl_cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, homes)) = cache.as_ref() {
            if at.elapsed() < WSL_CACHE_TTL {
                return homes.clone();
            }
        }
        let homes = discover_wsl_homes();
        *cache = Some((Instant::now(), homes.clone()));
        homes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup<'a>(
        manual: Option<&str>,
        env: Option<&str>,
        wsl_homes: &'a dyn Fn() -> Vec<WslHome>,
        exists: &'a dyn Fn(&Path) -> bool,
    ) -> Lookup<'a> {
        Lookup {
            manual: manual.map(PathBuf::from),
            env: env.map(PathBuf::from),
            home: Some(PathBuf::from("home")),
            wsl_homes,
            exists,
        }
    }

    fn no_wsl() -> Vec<WslHome> {
        panic!("WSL must not be probed when a Windows path matches")
    }

    #[test]
    fn manual_override_wins() {
        let exists = |_: &Path| true;
        let resolved = resolve_dir(
            Tool::Claude,
            &lookup(Some("manual"), Some("env"), &no_wsl, &exists),
        )
        .unwrap();
        assert_eq!(resolved.source, DirSource::Manual);
        assert_eq!(resolved.path, PathBuf::from("manual"));
    }

    #[test]
    fn env_beats_native() {
        let exists = |_: &Path| true;
        let resolved =
            resolve_dir(Tool::Codex, &lookup(None, Some("env"), &no_wsl, &exists)).unwrap();
        assert_eq!(resolved.source, DirSource::Env);
    }

    #[test]
    fn missing_override_falls_through_to_native() {
        let native = PathBuf::from("home").join(".claude");
        let exists = |path: &Path| path == native;
        let resolved = resolve_dir(
            Tool::Claude,
            &lookup(Some("gone"), Some("gone2"), &no_wsl, &exists),
        )
        .unwrap();
        assert_eq!(resolved.source, DirSource::Native);
        assert_eq!(resolved.path, native);
    }

    #[test]
    fn falls_back_to_wsl() {
        let wsl = || {
            vec![
                WslHome {
                    distro: "Ubuntu".into(),
                    home: PathBuf::from("wsl-a"),
                },
                WslHome {
                    distro: "Debian".into(),
                    home: PathBuf::from("wsl-b"),
                },
            ]
        };
        let target = PathBuf::from("wsl-b").join(".codex");
        let exists = |path: &Path| path == target;
        let resolved = resolve_dir(Tool::Codex, &lookup(None, None, &wsl, &exists)).unwrap();
        assert_eq!(
            resolved.source,
            DirSource::Wsl {
                distro: "Debian".into()
            }
        );
        assert!(resolved.is_wsl());
    }

    #[test]
    fn nothing_found() {
        fn empty() -> Vec<WslHome> {
            Vec::new()
        }
        let exists = |_: &Path| false;
        assert!(resolve_dir(Tool::Claude, &lookup(None, None, &empty, &exists)).is_none());
    }

    #[test]
    fn parses_distro_list() {
        let parsed = parse_wsl_distros(
            "Ubuntu\r\n\r\ndocker-desktop\r\ndocker-desktop-data\r\n Debian \r\n",
        );
        assert_eq!(parsed, vec!["Ubuntu".to_string(), "Debian".to_string()]);
    }
}
