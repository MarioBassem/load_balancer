use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
#[derive(Eq, PartialEq, Debug, Serialize, Deserialize)]
pub struct Server {
    pub url: String,
    pub name: String,
    pub disabled: bool,
    pub weight: u32,
    pub health_check_period: u32, // in seconds

    #[serde(skip_deserializing)]
    pub connections: u32,

    #[serde(skip_deserializing)]
    pub healthy: bool,
}

impl Ord for Server {
    fn cmp(&self, other: &Self) -> Ordering {
        if self.connections < other.connections {
            return Ordering::Less;
        }

        if self.connections == other.connections {
            return Ordering::Equal;
        }

        return Ordering::Greater;
    }
}
impl PartialOrd for Server {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.connections.partial_cmp(&other.connections)
    }
}