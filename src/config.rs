use serde::Deserialize;

#[derive(Clone, Deserialize)]
pub struct Target {
    pub name: String,
    pub address: String,
}

#[derive(Clone, Deserialize)]
pub struct AdminCommand {
    pub name: String,
    pub cmd: String,
    pub args: Vec<String>,
}

#[derive(Deserialize)]
pub struct AppConfig {
    pub targets: Vec<Target>,
    pub commands: Vec<AdminCommand>,
}