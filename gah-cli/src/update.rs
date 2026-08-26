//! Self-update: check GitHub for a newer release, swap in the new binary,
//! and restart any running systemd services.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const DEFAULT_REPO: &str = "orndorffgrant/gah";

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

pub async fn run(check_only: bool, repo: Option<&str>) -> Result<()> {
    let repo = repo.unwrap_or(DEFAULT_REPO);
    let current = env!("CARGO_PKG_VERSION");
    let target = current_target()?;

    println!("current: gah {current} ({target})");

    let release = latest_release(repo).await?;
    println!("latest:  {}", release.tag_name);

    if !is_newer(&release.tag_name, current) {
        println!("already up to date");
        return Ok(());
    }

    let asset = pick_asset(&release, target)
        .with_context(|| format!("no release asset for target {target}"))?;

    if check_only {
        println!("update available: {} ({})", release.tag_name, asset.name);
        println!("re-run without --check to apply");
        return Ok(());
    }

    println!("downloading {}", asset.name);
    let tarball = download(&asset.browser_download_url).await?;
    let new_bin = extract(&tarball)?;
    replace_current_exe(&new_bin)?;
    println!("updated to {}", release.tag_name);

    crate::systemd_service::restart_active_services()?;
    Ok(())
}

fn current_target() -> Result<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Ok("x86_64-unknown-linux-gnu"),
        "aarch64" => Ok("aarch64-unknown-linux-gnu"),
        other => bail!("unsupported architecture for self-update: {other}"),
    }
}

async fn latest_release(repo: &str) -> Result<Release> {
    let resp = reqwest::Client::new()
        .get(format!("https://api.github.com/repos/{repo}/releases/latest"))
        .header("User-Agent", "gah")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("failed to query GitHub releases API")?
        .error_for_status()
        .context("GitHub releases API returned an error")?;
    resp.json::<Release>()
        .await
        .context("failed to parse release JSON")
}

fn is_newer(latest_tag: &str, current: &str) -> bool {
    cmp_versions(parse_version(latest_tag), parse_version(current))
        == std::cmp::Ordering::Greater
}

fn parse_version(s: &str) -> Vec<u64> {
    s.trim_start_matches('v')
        .split('-')
        .next()
        .unwrap_or("")
        .split('.')
        .filter_map(|p| p.parse::<u64>().ok())
        .collect()
}

fn cmp_versions(a: Vec<u64>, b: Vec<u64>) -> std::cmp::Ordering {
    let n = a.len().max(b.len());
    (0..n)
        .map(|i| {
            a.get(i).copied().unwrap_or(0).cmp(&b.get(i).copied().unwrap_or(0))
        })
        .find(|o| *o != std::cmp::Ordering::Equal)
        .unwrap_or(std::cmp::Ordering::Equal)
}

fn pick_asset<'a>(release: &'a Release, target: &str) -> Option<&'a Asset> {
    release
        .assets
        .iter()
        .find(|a| a.name.contains(target) && a.name.ends_with(".tar.gz"))
}

async fn download(url: &str) -> Result<Vec<u8>> {
    let bytes = reqwest::Client::new()
        .get(url)
        .header("User-Agent", "gah")
        .send()
        .await
        .context("failed to start download")?
        .error_for_status()
        .context("download failed")?
        .bytes()
        .await
        .context("failed to read download")?;
    Ok(bytes.to_vec())
}

fn extract(tarball: &[u8]) -> Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!("gah-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let tar_path = dir.join("gah.tar.gz");
    std::fs::write(&tar_path, tarball)?;
    let out = Command::new("tar")
        .arg("-xzf")
        .arg(&tar_path)
        .arg("-C")
        .arg(&dir)
        .output()
        .context("failed to run tar")?;
    if !out.status.success() {
        bail!("tar extraction failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    let bin = dir.join("gah");
    if !bin.exists() {
        bail!("release archive did not contain a `gah` binary");
    }
    Ok(bin)
}

fn replace_current_exe(new_bin: &Path) -> Result<()> {
    let current = std::env::current_exe().context("could not determine current executable")?;
    let staged = current.with_extension("new");
    std::fs::copy(new_bin, &staged)
        .with_context(|| format!("failed to stage new binary at {}", staged.display()))?;
    let mode = std::fs::metadata(&current)?.permissions().mode();
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(mode))?;
    std::fs::rename(&staged, &current)
        .with_context(|| format!("failed to replace {}", current.display()))?;
    Ok(())
}