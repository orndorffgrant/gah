use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

fn build_start_args(
    unit: &str,
    description: &str,
    exe: &str,
    service_args: &[String],
    env_vars: &[String],
    working_dir: Option<&Path>,
) -> Vec<String> {
    let mut args = vec![
        "--unit".to_string(),
        unit.to_string(),
        "--description".to_string(),
        description.to_string(),
        "--service-type".to_string(),
        "exec".to_string(),
    ];
    if let Some(dir) = working_dir.filter(|dir| *dir != Path::new("/")) {
        args.push("--working-directory".to_string());
        args.push(dir.to_string_lossy().into_owned());
    }
    for var in env_vars {
        args.push("--setenv".to_string());
        args.push(var.clone());
    }
    args.push(exe.to_string());
    args.extend(service_args.iter().cloned());
    args
}

pub fn start_service(
    unit: &str,
    description: &str,
    service_args: &[String],
    env_vars: &[String],
) -> Result<()> {
    let current_exe = std::env::current_exe()
        .context("Failed to get current executable path")?;
    let exe = current_exe.to_string_lossy().into_owned();
    let working_dir = std::env::current_dir().ok();
    let args = build_start_args(
        unit,
        description,
        &exe,
        service_args,
        env_vars,
        working_dir.as_deref(),
    );

    let output = Command::new("systemd-run")
        .args(&args)
        .output()
        .with_context(|| format!("Failed to execute systemd-run {}", args.join(" ")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to start systemd service {}: {}", unit, stderr);
    }

    Ok(())
}

pub fn stop_service(unit: &str) -> Result<()> {
    systemctl(unit, "stop")
}

pub fn restart_service(unit: &str) -> Result<()> {
    systemctl(unit, "restart")
}

fn systemctl(unit: &str, action: &str) -> Result<()> {
    let output = Command::new("systemctl")
        .args([action, unit])
        .output()
        .with_context(|| format!("Failed to execute systemctl {} {}", action, unit))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to {} systemd service {}: {}", action, unit, stderr);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_args_place_options_before_command() {
        let service_args = vec![
            "run-webui".to_string(),
            "--bind".to_string(),
            "0.0.0.0:3000".to_string(),
        ];
        let env_vars = vec!["GAH_API_TOKEN=tok".to_string()];
        let args = build_start_args(
            "gah-webui",
            "GAH Web UI Service",
            "/usr/bin/gah",
            &service_args,
            &env_vars,
            Some(Path::new("/home/grant")),
        );

        assert_eq!(args[0], "--unit");
        assert_eq!(args[1], "gah-webui");
        let exe_pos = args
            .iter()
            .position(|a| a == "/usr/bin/gah")
            .expect("exe in args");
        for flag in ["--unit", "--description", "--service-type", "--working-directory", "--setenv"] {
            let pos = args.iter().position(|a| a == flag).expect(flag);
            assert!(pos < exe_pos, "{flag} must come before the command");
        }
        assert_eq!(&args[exe_pos + 1..], &service_args[..]);
    }

    #[test]
    fn start_args_skip_root_working_dir() {
        let args = build_start_args(
            "gah-api",
            "GAH API Service",
            "/usr/bin/gah",
            &["run-api".to_string()],
            &[],
            Some(Path::new("/")),
        );
        assert!(!args.contains(&"--working-directory".to_string()));
    }
}
