use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
#[derive(Eq, PartialEq, Debug, Serialize, Deserialize)]
pub(crate) struct Server {
    pub url: String,
    pub name: String,
    pub weight: u32,
    pub health_check_period: u64, // in seconds

    #[serde(skip_deserializing)]
    pub connections: u32,

    #[serde(skip_deserializing)]
    pub healthy: bool,
}

impl Ord for Server {
    fn cmp(&self, other: &Self) -> Ordering {
        if self.healthy && self.connections < other.connections
            || (self.healthy && self.connections == other.connections && self.weight > other.weight)
        {
            return Ordering::Less;
        }

        if self.healthy
            && other.healthy
            && self.connections == other.connections
            && self.weight == other.weight
        {
            return Ordering::Equal;
        }

        Ordering::Greater
    }
}
impl PartialOrd for Server {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
