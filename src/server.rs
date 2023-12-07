use std::cmp::Ordering;

#[derive(Eq, PartialEq, Debug)]
pub struct Server {
    pub url: String,
    pub name: String,
    pub connections: u32,
}

impl Ord for Server {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
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
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.connections.partial_cmp(&other.connections)
    }
}
