use anyhow::Result;
use rusqlite::Connection;

use crate::store::now_ms;

use super::{schema, seed};

pub(crate) fn initialize_schema(conn: &Connection) -> Result<()> {
    schema::initialize_base_schema(conn)?;
    seed::seed_builtins(conn)?;
    seed::seed_opencode_builtin_agent(conn, now_ms())?;
    Ok(())
}
