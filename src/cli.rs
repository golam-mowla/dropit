use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
#[command(
    name = "dropit",
    about = "Drop It — Fast, lightweight, secure local WiFi file transfer tool and anonymous messenger",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    /// Send one or more files to a discovered device on your local WiFi
    Send(SendArgs),

    /// Start Drop It receiver mode
    Receive(ReceiveArgs),

    /// Scan and list nearby Drop It devices on the local network
    Discover(DiscoverArgs),
}

#[derive(Args, Debug, Clone)]
pub struct SendArgs {
    /// Name, IP, or Device ID of the target device
    #[arg(required = true)]
    pub target: String,

    /// Paths of the files to send
    #[arg(required = true)]
    pub files: Vec<PathBuf>,

    /// Verify blake3 hash after transfer
    #[arg(long, default_value_t = true)]
    pub verify_hash: bool,

    /// Target server port (default: 53420)
    #[arg(long, default_value_t = 53420)]
    pub port: u16,
}

#[derive(Args, Debug, Clone)]
pub struct ReceiveArgs {
    /// Port to bind the HTTP TLS server (default: 53420)
    #[arg(long, default_value_t = 53420)]
    pub port: u16,

    /// Destination directory to save received files
    #[arg(long, default_value = ".")]
    pub dir: PathBuf,

    /// Auto-accept transfers from trusted devices
    #[arg(long, default_value_t = false)]
    pub auto_accept: bool,
}

#[derive(Args, Debug, Clone)]
pub struct DiscoverArgs {
    /// Timeout in seconds to scan for devices
    #[arg(long, default_value_t = 3)]
    pub timeout: u64,

    /// UDP discovery port (default: 53421)
    #[arg(long, default_value_t = 53421)]
    pub port: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parse_send() {
        let args = Cli::parse_from([
            "dropit",
            "send",
            "192.168.1.5",
            "file.txt",
            "--port",
            "53420",
        ]);
        if let Commands::Send(send_args) = args.command {
            assert_eq!(send_args.target, "192.168.1.5");
            assert_eq!(send_args.files, vec![PathBuf::from("file.txt")]);
            assert_eq!(send_args.port, 53420);
            assert!(send_args.verify_hash);
        } else {
            panic!("Expected Send command");
        }
    }

    #[test]
    fn test_cli_parse_receive() {
        let args = Cli::parse_from(["dropit", "receive", "--port", "53420", "--dir", "downloads"]);
        if let Commands::Receive(rcv_args) = args.command {
            assert_eq!(rcv_args.port, 53420);
            assert_eq!(rcv_args.dir, PathBuf::from("downloads"));
        } else {
            panic!("Expected Receive command");
        }
    }

    #[test]
    fn test_cli_parse_discover() {
        let args = Cli::parse_from(["dropit", "discover", "--timeout", "5"]);
        if let Commands::Discover(disc_args) = args.command {
            assert_eq!(disc_args.timeout, 5);
        } else {
            panic!("Expected Discover command");
        }
    }
}
