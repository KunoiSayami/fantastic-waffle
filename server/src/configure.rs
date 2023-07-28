pub mod v1 {
    use anyhow::anyhow;
    use serde_derive::Deserialize;
    use std::collections::HashMap;
    use std::fmt::Display;
    use std::path::Path;
    use tap::TapFallible;
    use tokio::fs::read_to_string;

    #[derive(Clone, Debug, Deserialize)]
    pub struct AuthEntry {
        token: String,
        path: Vec<String>,
    }

    impl AuthEntry {
        pub fn token(&self) -> &str {
            &self.token
        }
        pub fn path(&self) -> &Vec<String> {
            &self.path
        }
    }

    #[derive(Clone, Debug, Deserialize)]
    pub struct Server {
        host: String,
        port: u16,
    }

    impl Server {
        pub fn host(&self) -> &str {
            &self.host
        }
        pub fn port(&self) -> u16 {
            self.port
        }

        pub fn get_bind(&self) -> String {
            format!("{}:{}", self.host, self.port)
        }
    }

    #[derive(Clone, Debug, Deserialize)]
    pub struct Configure {
        working_directory: String,
        server: Server,
        auth_entry: Vec<AuthEntry>,
    }

    impl Configure {
        pub fn server(&self) -> &Server {
            &self.server
        }
        pub fn auth_entry(&self) -> &Vec<AuthEntry> {
            &self.auth_entry
        }

        pub async fn load<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
            let file = read_to_string(path)
                .await
                .map_err(|e| anyhow!("Unable to load configure file: {:?}", e))?;
            toml::from_str(&file)
                .map_err(|e| anyhow!("Unable to deserialize configure file: {:?}", e))
        }
        pub fn working_directory(&self) -> &str {
            &self.working_directory
        }

        pub fn parse_host_and_port(&self, host: Option<&String>, port: Option<&u16>) -> String {
            if host.is_some() && port.is_some() {
                return format!("{}:{}", host.unwrap(), port.unwrap());
            }
            if let Some(host) = host {
                return format!("{}:{}", host, self.server().port());
            }
            if let Some(port) = port {
                return format!("{}:{}", self.server().host(), port);
            }
            self.server().get_bind()
        }

        pub fn build_hashmap(&self) -> HashMap<String, Vec<String>> {
            let mut m = HashMap::new();
            for auth_entry in &self.auth_entry {
                m.insert(auth_entry.token().to_string(), auth_entry.path().clone());
            }
            m
        }
    }
}

pub use v1 as current;
