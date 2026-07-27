pub mod lmdb;

pub use lmdb::LmdbStore;

#[derive(Debug, PartialEq, Eq)]
pub enum AddEventResult {
    New,
    Duplicate,
    Replaced(usize),
}
