use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct Mock {
    hello: u32,
    world: String,
}
