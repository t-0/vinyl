use super::Scan;
use anyhow::Result;
pub enum IndexMsg {
    Progress { done: usize, total: usize },
    Done(Result<Scan>),
}
