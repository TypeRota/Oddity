//! 启动组件安装计划. 系统变更通过明确的包管理器命令授权执行.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const UMU_VERSION: &str = "1.4.4";
pub const PROTON_VERSION: &str = "GE-Proton11-5-x86_64";
const UMU_URL: &str = "https://github.com/Open-Wine-Components/umu-launcher/releases/download/1.4.4/umu-launcher-1.4.4-zipapp.tar";
const UMU_HASH: &str = "eb590691841f7fad3fc3ad8fd5db4ccb87849fe7948e62b28ece7a4ee48cc851";
const PROTON_URL: &str = "https://github.com/GloriousEggroll/proton-ge-custom/releases/download/GE-Proton11-5/GE-Proton11-5-x86_64.tar.gz";
const PROTON_HASH: &str = "de43c4b25f3c047db49b96c44d84759952c5a01332a68805a09e69f95dc38a75";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Receipt {
    pub seen: bool,
    pub proton: Option<PathBuf>,
}
impl Receipt {
    pub fn load(data: &Path) -> Result<Self> {
        let path = data.join("runtime/setup.json");
        if !path.exists() {
            return Ok(Self::default());
        }
        let mut receipt: Self = serde_json::from_slice(&fs::read(path)?)?;
        receipt.proton = receipt.proton.filter(|path| {
            path.is_absolute()
                && path.join("proton").is_file()
                && path.join("toolmanifest.vdf").is_file()
        });
        Ok(receipt)
    }
    pub fn save(&self, data: &Path) -> Result<()> {
        fs::create_dir_all(data.join("runtime"))?;
        let path = data.join("runtime/setup.json");
        let mut temp = crate::paths::TemporaryFile::new(&path)?;
        temp.file.write_all(&serde_json::to_vec_pretty(self)?)?;
        temp.commit(&path, &|| false)?;
        Ok(())
    }
}

pub fn executable(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|p| p.join(name))
            .find(|p| {
                p.is_file() && fs::metadata(p).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
            })
    })
}
pub fn system_umu() -> Option<PathBuf> {
    [
        "/usr/bin/umu-run",
        "/usr/local/bin/umu-run",
        "/run/current-system/sw/bin/umu-run",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|p| p.is_file() && fs::metadata(p).is_ok_and(|m| m.permissions().mode() & 0o111 != 0))
}
pub fn umu(data: &Path) -> Option<PathBuf> {
    executable("umu-run").or_else(|| {
        let path = data.join("runtime/umu/umu-run");
        (path.is_file() && fs::metadata(&path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0))
            .then_some(path)
    })
}
pub fn steam_tools(home: &Path) -> PathBuf {
    [
        home.join(".steam/root"),
        home.join(".steam/steam"),
        home.join(".local/share/Steam"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
    ]
    .into_iter()
    .find(|p| p.is_dir())
    .unwrap_or_else(|| home.join(".local/share/Steam"))
    .join("compatibilitytools.d")
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub umu: PathBuf,
    pub proton: PathBuf,
    pub install_umu: bool,
    pub install_proton: bool,
    pub system_command: Option<Vec<String>>,
    pub umu_version: String,
    pub log: PathBuf,
}

pub fn plan(
    data: &Path,
    home: &Path,
    all_users: bool,
    local_proton: Option<&Path>,
    os_release: &str,
) -> Result<Plan> {
    let existing = if all_users { system_umu() } else { umu(data) };
    let install_umu = existing.is_none();
    let system_command = if all_users && install_umu {
        if executable("pkexec").is_none() {
            bail!("缺少 pkexec, 请先安装提供该命令的 polkit 软件包");
        }
        Some(system_install(os_release)?)
    } else {
        None
    };
    if install_umu && !all_users {
        let python =
            executable("python3").context("缺少 Python 3, 请先安装 Python 3.10 或更新版本")?;
        let status = Command::new(python)
            .args(["-c", "import sys; sys.exit(sys.version_info < (3, 10))"])
            .status()?;
        if !status.success() {
            bail!("umu 需要 Python 3.10 或更新版本");
        }
    }
    let proton = local_proton
        .map(Path::to_path_buf)
        .unwrap_or_else(|| steam_tools(home).join(PROTON_VERSION));
    if local_proton.is_some()
        && (!proton.is_absolute()
            || !proton.join("proton").is_file()
            || !proton.join("toolmanifest.vdf").is_file())
    {
        bail!("请选择包含 proton 和 toolmanifest.vdf 的 Proton 绝对路径");
    }
    let install_proton =
        !proton.join("proton").is_file() || !proton.join("toolmanifest.vdf").is_file();
    if install_proton && proton.exists() {
        bail!("Proton 目标目录已存在但不完整, 请检查或移走该目录后重试");
    }
    Ok(Plan {
        umu: existing.unwrap_or_else(|| {
            if all_users {
                PathBuf::from("/usr/bin/umu-run")
            } else {
                data.join("runtime/umu/umu-run")
            }
        }),
        proton,
        install_umu,
        install_proton,
        system_command,
        umu_version: if install_umu && !all_users {
            UMU_VERSION.into()
        } else {
            "系统或已有版本".into()
        },
        log: data.join("logs/runtime-setup.log"),
    })
}

pub fn system_install(os_release: &str) -> Result<Vec<String>> {
    let fields: std::collections::HashMap<_, _> = os_release
        .lines()
        .filter_map(|line| line.split_once('='))
        .collect();
    let id = fields.get("ID").copied().unwrap_or("").trim_matches('"');
    let like = fields
        .get("ID_LIKE")
        .copied()
        .unwrap_or("")
        .trim_matches('"');
    let ids: Vec<_> = std::iter::once(id).chain(like.split_whitespace()).collect();
    // 仅使用发行版仓库, 缺少软件包时保留包管理器错误供用户处理.
    let command: &[&str] = if ids.contains(&"arch") {
        &[
            "/usr/bin/pacman",
            "-S",
            "--needed",
            "--noconfirm",
            "umu-launcher",
        ]
    } else if ids.contains(&"fedora") {
        &["/usr/bin/dnf", "install", "-y", "umu-launcher"]
    } else if ids.contains(&"debian") || ids.contains(&"ubuntu") {
        &["/usr/bin/apt-get", "install", "-y", "umu-launcher"]
    } else if ids.contains(&"opensuse") || ids.contains(&"opensuse-tumbleweed") {
        &[
            "/usr/bin/zypper",
            "--non-interactive",
            "install",
            "umu-launcher",
        ]
    } else {
        bail!("此发行版暂不支持自动为全部用户安装 umu, 请使用当前用户安装或手动安装");
    };
    Ok(command.iter().map(|s| s.to_string()).collect())
}

#[derive(Clone, Copy, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Component {
    #[default]
    Umu,
    Proton,
    Runtime,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub component: Component,
    pub phase: String,
    pub detail: String,
    pub downloaded: u64,
    pub total: u64,
}

pub fn download(
    url: &str,
    hash: &str,
    target: &Path,
    progress: &mut impl FnMut(Progress),
) -> Result<()> {
    if target.is_file() {
        let mut input = File::open(target)?;
        let mut hasher = Sha256::new();
        let mut buffer = [0; 128 * 1024];
        loop {
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        if hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
            == hash
        {
            return Ok(());
        }
        fs::remove_file(target)?;
    }
    let mut response = ureq::get(url)
        .header("User-Agent", crate::USER_AGENT)
        .call()?;
    let total = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let part = target.with_extension("part");
    let mut output = File::create(&part)?;
    let mut input = response.body_mut().as_reader();
    let mut hasher = Sha256::new();
    let mut buffer = [0; 128 * 1024];
    let mut downloaded = 0;
    let mut last = std::time::Instant::now();
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        output.write_all(&buffer[..count])?;
        hasher.update(&buffer[..count]);
        downloaded += count as u64;
        if last.elapsed().as_millis() >= 250 {
            progress(Progress {
                component: Component::Umu,
                phase: "download".into(),
                detail: target.file_name().unwrap().to_string_lossy().into(),
                downloaded,
                total,
            });
            last = std::time::Instant::now();
        }
    }
    if hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
        != hash
    {
        fs::remove_file(&part)?;
        bail!("下载文件校验失败, 请重试");
    }
    output.sync_all()?;
    fs::rename(part, target)?;
    Ok(())
}

pub fn install(data: &Path, plan: &Plan, mut progress: impl FnMut(Progress)) -> Result<PathBuf> {
    fs::create_dir_all(data.join("runtime/downloads"))?;
    fs::create_dir_all(plan.log.parent().unwrap())?;
    let log = File::create(&plan.log)?;
    writeln!(
        &log,
        "umu: {}\nProton: {}",
        plan.umu.display(),
        plan.proton.display()
    )?;
    let mut stage = |detail: &str| {
        progress(Progress {
            phase: "prepare".into(),
            detail: detail.into(),
            ..Default::default()
        })
    };
    stage("检查启动组件");
    if let Some(command) = &plan.system_command {
        writeln!(&log, "授权操作: pkexec {}", command.join(" "))?;
        stage("等待管理员授权并安装 umu");
        let status = Command::new("pkexec")
            .args(command)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log.try_clone()?)
            .status()?;
        if matches!(status.code(), Some(126 | 127)) {
            bail!("管理员授权已取消或被拒绝");
        }
        if !status.success() {
            bail!("系统组件安装失败, 请查看日志: {}", plan.log.display());
        }
    }
    if plan.install_umu && plan.system_command.is_none() {
        let archive = data.join("runtime/downloads/umu-1.4.4.tar");
        download(UMU_URL, UMU_HASH, &archive, &mut progress)?;
        let temp = tempfile::tempdir_in(data.join("runtime"))?;
        tar::Archive::new(File::open(archive)?).unpack(temp.path())?;
        let executable = temp.path().join("umu/umu-run");
        if !executable.is_file() {
            bail!("umu 分发包缺少 umu-run");
        }
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))?;
        fs::create_dir_all(plan.umu.parent().unwrap())?;
        fs::rename(executable, &plan.umu)?;
    }
    let umu = &plan.umu;
    if !umu.is_file() {
        bail!("安装完成后未找到 umu-run");
    }
    let status = Command::new(umu)
        .arg("--version")
        .stdout(log.try_clone()?)
        .stderr(log.try_clone()?)
        .status()?;
    if !status.success() {
        bail!("umu 无法运行, 请查看日志: {}", plan.log.display());
    }
    progress(Progress {
        component: Component::Proton,
        phase: "prepare".into(),
        detail: "准备 Proton".into(),
        ..Default::default()
    });
    if plan.install_proton {
        let archive = data.join("runtime/downloads/GE-Proton11-5-x86_64.tar.gz");
        download(PROTON_URL, PROTON_HASH, &archive, &mut |mut value| {
            value.component = Component::Proton;
            progress(value);
        })?;
        progress(Progress {
            component: Component::Proton,
            phase: "extract".into(),
            detail: "解压 Proton".into(),
            ..Default::default()
        });
        let parent = plan.proton.parent().context("Proton 目标目录无效")?;
        fs::create_dir_all(parent)?;
        let temp = tempfile::tempdir_in(parent)?;
        tar::Archive::new(flate2::read::GzDecoder::new(File::open(archive)?))
            .unpack(temp.path())?;
        let source = temp.path().join(PROTON_VERSION);
        if !source.join("proton").is_file() || !source.join("toolmanifest.vdf").is_file() {
            bail!("Proton 分发包不完整");
        }
        fs::rename(source, &plan.proton)?;
    }
    progress(Progress {
        component: Component::Runtime,
        phase: "runtime".into(),
        detail: "准备 Steam Linux Runtime, 详情见安装日志".into(),
        ..Default::default()
    });
    // 用 wineboot 完成初始化, 同时准备共享运行时.
    let prefix = tempfile::tempdir_in(data.join("runtime"))?;
    initialize(umu, &plan.proton, prefix.path(), &plan.log)?;
    Receipt {
        seen: true,
        proton: Some(plan.proton.clone()),
    }
    .save(data)?;
    Ok(plan.proton.clone())
}

pub fn initialize(umu: &Path, proton: &Path, prefix: &Path, log: &Path) -> Result<()> {
    let output = fs::OpenOptions::new().create(true).append(true).open(log)?;
    let status = Command::new(umu)
        .args(["wineboot", "-u"])
        .env("PROTON_VERB", "waitforexitandrun")
        .env("PROTONPATH", proton)
        .env("WINEPREFIX", prefix)
        .env("GAMEID", "0")
        .stdin(Stdio::null())
        .stdout(output.try_clone()?)
        .stderr(output)
        .status()?;
    if !status.success() {
        bail!(
            "wineboot 初始化失败 (退出码 {}), 请查看日志: {}",
            status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "进程被信号终止".into()),
            log.display()
        );
    }
    if !prefix.join("system.reg").is_file() {
        bail!("初始化未生成 Windows 注册表, 请查看日志: {}", log.display());
    }
    Ok(())
}
