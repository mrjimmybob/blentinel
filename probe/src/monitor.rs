use common::models::{Health, ResourceStatus};
use surge_ping::{Client, Config, PingIdentifier, PingSequence};
use std::time::Duration;
use reqwest::Client as HttpClient;
use std::net::IpAddr;
use tokio::net::TcpStream;

pub struct Monitor {
    icmp_client: Client,
    http_client: HttpClient,
}

impl Monitor {
    pub fn new() -> Self {
        let icmp_config = Config::default();
        Self {
            icmp_client: Client::new(&icmp_config).expect("Failed to create ICMP client"),
            http_client: HttpClient::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .expect("Failed to create HTTP client"),
        }
    }

    pub async fn check_ping(&self, name: String, target: String) -> ResourceStatus {
        let ip: IpAddr = match target.parse() {
            Ok(addr) => addr,
            Err(_) => return self.error_status(name, target, "Invalid IP address"),
        };

        let payload = [0u8; 8];
        let mut pinger = self.icmp_client.pinger(ip, PingIdentifier(0)).await;
        
        match pinger.ping(PingSequence(0), &payload).await {
            Ok((_, duration)) => ResourceStatus {
                name,
                resource_type: "ping".to_string(),
                target,
                status: Health::Up,
                message: format!("Responded in {:?}", duration),
                latency_ms: Some(duration.as_millis() as u64),
            },
            Err(e) => self.error_status(name, target, &format!("Ping failed: {}", e)),
        }
    }

    pub async fn check_http(&self, name: String, target: String) -> ResourceStatus {
        match self.http_client.get(&target).send().await {
            Ok(resp) if resp.status().is_success() => ResourceStatus {
                name,
                resource_type: "http".to_string(),
                target,
                status: Health::Up,
                message: format!("HTTP {}", resp.status()),
                latency_ms: None, // Optional: track time if needed
            },
            Ok(resp) => self.error_status(name, target, &format!("HTTP Error: {}", resp.status())),
            Err(e) => self.error_status(name, target, &format!("Connection failed: {}", e)),
        }
    }

    pub async fn check_tcp(&self, name: String, target: String) -> ResourceStatus {
        let start = std::time::Instant::now();
        
        // target should be "ip:port" e.g., "192.168.1.50:1433"
        let timeout_duration = Duration::from_secs(3);
        
        let connection = tokio::time::timeout(
            timeout_duration, 
            TcpStream::connect(&target)
        ).await;

        match connection {
            Ok(Ok(_)) => ResourceStatus {
                name,
                resource_type: "tcp".to_string(),
                target,
                status: Health::Up,
                message: "Port Open".to_string(),
                latency_ms: Some(start.elapsed().as_millis() as u64),
            },
            Ok(Err(e)) => self.error_status(name, target, &format!("Connection Refused: {}", e)),
            Err(_) => self.error_status(name, target, "Connection Timeout"),
        }
    }
    
    pub fn error_status(&self, name: String, target: String, msg: &str) -> ResourceStatus {
        ResourceStatus {
            name,
            resource_type: "error".to_string(),
            target,
            status: Health::Down,
            message: msg.to_string(),
            latency_ms: None,
        }
    }
}