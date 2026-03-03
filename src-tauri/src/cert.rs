use std::fs;
use std::path::PathBuf;
use std::process::Command;

use crate::models::CertStatus;
use crate::utils::{get_cert_path, get_key_path};

// ==================== Certificate Management ====================

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

fn certutil_output(args: &[&str]) -> Result<std::process::Output, String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        Command::new("certutil")
            .creation_flags(CREATE_NO_WINDOW)
            .args(args)
            .output()
            .map_err(|e| format!("certutil execution failed: {}", e))
    }
    #[cfg(not(windows))]
    {
        Command::new("certutil")
            .args(args)
            .output()
            .map_err(|e| format!("certutil execution failed: {}", e))
    }
}

/// Generate a self-signed CA certificate if one does not exist
pub fn ensure_cert_exists() -> Result<(PathBuf, PathBuf), String> {
    let cert_path = get_cert_path();
    let key_path = get_key_path();

    if cert_path.exists() && key_path.exists() {
        return Ok((cert_path, key_path));
    }

    // Generate certificate with rcgen
    use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, SanType};
    use std::net::IpAddr;

    let mut params = CertificateParams::default();
    params
        .distinguished_name
        .push(DnType::CommonName, "AG Proxy Local CA");
    params
        .distinguished_name
        .push(DnType::OrganizationName, "AG Proxy Manager");
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.subject_alt_names = vec![
        SanType::DnsName("localhost".try_into().unwrap()),
        SanType::IpAddress("127.0.0.1".parse::<IpAddr>().unwrap()),
    ];
    // Valid for 10 years
    params.not_before = rcgen::date_time_ymd(2024, 1, 1);
    params.not_after = rcgen::date_time_ymd(2034, 12, 31);

    let key_pair = KeyPair::generate().map_err(|e| format!("Key generation failed: {}", e))?;
    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| format!("Certificate generation failed: {}", e))?;

    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();

    fs::write(&cert_path, &cert_pem).map_err(|e| format!("Failed to save certificate: {}", e))?;
    fs::write(&key_path, &key_pem).map_err(|e| format!("Failed to save private key: {}", e))?;

    Ok((cert_path, key_path))
}

#[tauri::command]
pub fn import_cert() -> Result<String, String> {
    // Ensure certificate exists first
    let (cert_path, _) = ensure_cert_exists()?;
    let cert_str = cert_path.to_string_lossy().to_string();

    // Import to system trust store via certutil (requires admin privileges)
    let output = certutil_output(&["-addstore", "-f", "Root", &cert_str])?;

    if output.status.success() {
        Ok("✅ Certificate imported to system trust store".to_string())
    } else {
        Err("❌ Certificate import failed. Please run as administrator.".to_string())
    }
}

#[tauri::command]
pub fn remove_cert() -> Result<String, String> {
    let output = certutil_output(&["-delstore", "Root", "AG Proxy Local CA"])?;

    if output.status.success() {
        Ok("✅ Certificate removed from system trust store".to_string())
    } else {
        Err("❌ Certificate removal failed. Please run as administrator.".to_string())
    }
}

#[tauri::command]
pub fn check_cert_status() -> CertStatus {
    let cert_path = get_cert_path();

    let installed = certutil_output(&["-verifystore", "Root", "AG Proxy Local CA"])
        .map(|o| o.status.success())
        .unwrap_or(false);

    CertStatus {
        installed,
        cert_path: cert_path.to_string_lossy().to_string(),
    }
}
