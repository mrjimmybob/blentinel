#![cfg(feature = "ssr")]

use crate::config::SmtpConfig;
use anyhow::Result;

#[derive(Debug, Clone, Copy)]
pub enum AlertSeverity {
    Critical,
    Warning,
    Info,
}

impl AlertSeverity {
    pub fn as_str(&self) -> &str {
        match self {
            AlertSeverity::Critical => "CRITICAL",
            AlertSeverity::Warning => "WARNING",
            AlertSeverity::Info => "INFO",
        }
    }
}

pub struct AlertEmail {
    pub to: Vec<String>,
    pub subject: String,
    pub body: String,
}

/// Send an alert email using configured SMTP settings
pub async fn send_alert_email(smtp_config: &SmtpConfig, email: AlertEmail) -> Result<()> {
    if smtp_config.server.is_empty() {
        anyhow::bail!("SMTP server not configured");
    }

    // Use tokio::task::spawn_blocking to run sync SMTP code
    let server = smtp_config.server.clone();
    let port = smtp_config.port;
    let username = smtp_config.username.clone();
    let password = smtp_config.password.clone();
    let from = smtp_config.from.clone();

    tokio::task::spawn_blocking(move || {
        use lettre::{Message, SmtpTransport, Transport};
        use lettre::transport::smtp::authentication::Credentials;

        // Build message with all recipients
        let mut message_builder = Message::builder()
            .from(from.parse().map_err(|e| anyhow::anyhow!("Invalid from address: {}", e))?);

        for recipient in &email.to {
            message_builder = message_builder
                .to(recipient.parse().map_err(|e| anyhow::anyhow!("Invalid recipient {}: {}", recipient, e))?);
        }

        let message = message_builder
            .subject(&email.subject)
            .body(email.body)
            .map_err(|e| anyhow::anyhow!("Failed to build message: {}", e))?;

        // Build SMTP transport
        let mailer = if !username.is_empty() && !password.is_empty() {
            let creds = Credentials::new(username, password);
            SmtpTransport::relay(&server)
                .map_err(|e| anyhow::anyhow!("Failed to create SMTP transport: {}", e))?
                .port(port)
                .credentials(creds)
                .build()
        } else {
            // No authentication
            SmtpTransport::relay(&server)
                .map_err(|e| anyhow::anyhow!("Failed to create SMTP transport: {}", e))?
                .port(port)
                .build()
        };

        // Send
        mailer.send(&message)
            .map_err(|e| anyhow::anyhow!("Failed to send email: {}", e))?;

        Ok::<(), anyhow::Error>(())
    })
    .await
    .map_err(|e| anyhow::anyhow!("SMTP task panicked: {}", e))??;

    Ok(())
}

pub fn format_resource_down_email(
    company_id: &str,
    probe_name: &str,
    hostname: &str,
    site: &str,
    resource_name: &str,
    resource_type: &str,
    target: &str,
    message: &str,
) -> AlertEmail {
    let subject = format!(
        "[BLENTINEL][{}][{}][{}] {} DOWN",
        AlertSeverity::Critical.as_str(),
        company_id,
        site,
        resource_name
    );

    let body = format!(
        "╔═══════════════════════════════════════════════════════════════════════════╗\n\
         ║                         ⚠️  RESOURCE DOWN ALERT                          ║\n\
         ╚═══════════════════════════════════════════════════════════════════════════╝\n\
        \n\
        SEVERITY:     {}\n\
        \n\
        OPERATIONAL CONTEXT:\n\
        ────────────────────\n\
        Company:      {}\n\
        Site:         {}\n\
        Hostname:     {}\n\
        Probe:        {}\n\
        \n\
        RESOURCE DETAILS:\n\
        ────────────────\n\
        Name:         {}\n\
        Type:         {}\n\
        Target:       {}\n\
        Status:       DOWN ❌\n\
        \n\
        CHECK MESSAGE:\n\
        ────────────────\n\
        {}\n\
        \n\
        TIMESTAMP:\n\
        ────────────────\n\
        {}\n\
        \n\
        ═══════════════════════════════════════════════════════════════════════════\n\
        This resource has gone down and requires immediate attention.\n\
        Check network connectivity, service status, and system logs.\n\
        ═══════════════════════════════════════════════════════════════════════════\n\
        ",
        AlertSeverity::Critical.as_str(),
        company_id,
        site,
        hostname,
        probe_name,
        resource_name,
        resource_type,
        target,
        message,
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
    );

    AlertEmail {
        to: vec![],
        subject,
        body,
    }
}

pub fn format_resource_recovery_email(
    company_id: &str,
    probe_name: &str,
    hostname: &str,
    site: &str,
    resource_name: &str,
    resource_type: &str,
    target: &str,
) -> AlertEmail {
    let subject = format!(
        "[BLENTINEL][{}][{}][{}] {} RECOVERED",
        AlertSeverity::Info.as_str(),
        company_id,
        site,
        resource_name
    );

    let body = format!(
        "╔═══════════════════════════════════════════════════════════════════════════╗\n\
         ║                      ✅  RESOURCE RECOVERY ALERT                         ║\n\
         ╚═══════════════════════════════════════════════════════════════════════════╝\n\
        \n\
        SEVERITY:     {}\n\
        \n\
        OPERATIONAL CONTEXT:\n\
        ────────────────────\n\
        Company:      {}\n\
        Site:         {}\n\
        Hostname:     {}\n\
        Probe:        {}\n\
        \n\
        RESOURCE DETAILS:\n\
        ────────────────\n\
        Name:         {}\n\
        Type:         {}\n\
        Target:       {}\n\
        Status:       UP ✅\n\
        \n\
        TIMESTAMP:\n\
        ────────────────\n\
        {}\n\
        \n\
        ═══════════════════════════════════════════════════════════════════════════\n\
        This resource has recovered and is now operational.\n\
        No further action required unless the issue recurs.\n\
        ═══════════════════════════════════════════════════════════════════════════\n\
        ",
        AlertSeverity::Info.as_str(),
        company_id,
        site,
        hostname,
        probe_name,
        resource_name,
        resource_type,
        target,
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
    );

    AlertEmail {
        to: vec![],
        subject,
        body,
    }
}

pub fn format_threshold_email(
    company_id: &str,
    probe_name: &str,
    hostname: &str,
    site: &str,
    resource_name: &str,
    metric_type: &str,
    current_value: f64,
    threshold: u32,
) -> AlertEmail {
    let subject = format!(
        "[BLENTINEL][{}][{}][{}] {} {}%",
        AlertSeverity::Warning.as_str(),
        company_id,
        site,
        metric_type.to_uppercase(),
        current_value as u32
    );

    let body = format!(
        "╔═══════════════════════════════════════════════════════════════════════════╗\n\
         ║                      ⚠️  THRESHOLD WARNING ALERT                         ║\n\
         ╚═══════════════════════════════════════════════════════════════════════════╝\n\
        \n\
        SEVERITY:     {}\n\
        \n\
        OPERATIONAL CONTEXT:\n\
        ────────────────────\n\
        Company:      {}\n\
        Site:         {}\n\
        Hostname:     {}\n\
        Probe:        {}\n\
        \n\
        RESOURCE DETAILS:\n\
        ────────────────\n\
        Resource:     {}\n\
        Metric:       {}\n\
        Current:      {:.1}%\n\
        Threshold:    {}%\n\
        Status:       EXCEEDED ⚠️\n\
        \n\
        TIMESTAMP:\n\
        ────────────────\n\
        {}\n\
        \n\
        ═══════════════════════════════════════════════════════════════════════════\n\
        This resource has exceeded the configured threshold.\n\
        Investigate capacity, clean up space, or adjust threshold if appropriate.\n\
        ═══════════════════════════════════════════════════════════════════════════\n\
        ",
        AlertSeverity::Warning.as_str(),
        company_id,
        site,
        hostname,
        probe_name,
        resource_name,
        metric_type.to_uppercase(),
        current_value,
        threshold,
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
    );

    AlertEmail {
        to: vec![],
        subject,
        body,
    }
}

pub fn format_probe_expiry_email(
    company_id: &str,
    probe_id: &str,
    probe_name: &str,
    last_seen: &str,
    timeout_secs: u64,
) -> AlertEmail {
    let subject = format!(
        "[BLENTINEL][{}][{}] PROBE {} EXPIRED",
        AlertSeverity::Critical.as_str(),
        company_id,
        probe_name
    );

    let body = format!(
        "╔═══════════════════════════════════════════════════════════════════════════╗\n\
         ║                        🔴  PROBE EXPIRY ALERT                            ║\n\
         ╚═══════════════════════════════════════════════════════════════════════════╝\n\
        \n\
        SEVERITY:     {}\n\
        \n\
        OPERATIONAL CONTEXT:\n\
        ────────────────────\n\
        Company:      {}\n\
        Probe Name:   {}\n\
        Probe ID:     {}\n\
        \n\
        EXPIRY DETAILS:\n\
        ────────────────\n\
        Last Seen:    {}\n\
        Timeout:      {} seconds\n\
        Status:       EXPIRED ❌\n\
        \n\
        TIMESTAMP:\n\
        ────────────────\n\
        {}\n\
        \n\
        ═══════════════════════════════════════════════════════════════════════════\n\
        This probe has not reported within the configured timeout period.\n\
        \n\
        TROUBLESHOOTING STEPS:\n\
        1. Check if the probe machine is powered on and connected to network\n\
        2. Verify the Blentinel probe service is running\n\
        3. Check firewall rules and network connectivity to hub\n\
        4. Review probe logs for errors\n\
        ═══════════════════════════════════════════════════════════════════════════\n\
        ",
        AlertSeverity::Critical.as_str(),
        company_id,
        probe_name,
        probe_id,
        last_seen,
        timeout_secs,
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
    );

    AlertEmail {
        to: vec![],
        subject,
        body,
    }
}
