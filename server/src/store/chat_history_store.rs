use crate::{config::Config, models::OpenAIMessage, support::time::now_unix};
use rusqlite::{Connection, OptionalExtension, params};
use std::{fs, path::PathBuf, sync::Arc};

#[derive(Clone, Debug)]
pub struct ChatHistoryStore {
    db_path: PathBuf,
}

impl ChatHistoryStore {
    pub fn new(config: Arc<Config>) -> Result<Self, String> {
        let store = Self {
            db_path: config.chat_history_sqlite_path(),
        };
        store.init()?;
        Ok(store)
    }

    pub fn save_messages(
        &self,
        response_id: &str,
        messages: &[OpenAIMessage],
    ) -> Result<(), String> {
        let conn = self.connect()?;
        let updated_at = now_unix() as i64;
        let messages_json = serde_json::to_string(messages)
            .map_err(|err| format!("encode chat history messages failed: {err}"))?;

        conn.execute(
            "INSERT INTO chat_history (response_id, messages_json, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(response_id) DO UPDATE SET
               messages_json = excluded.messages_json,
               updated_at = excluded.updated_at",
            params![response_id, messages_json, updated_at],
        )
        .map_err(|err| format!("save chat history failed: {err}"))?;

        Ok(())
    }

    pub fn load_messages(&self, response_id: &str) -> Result<Option<Vec<OpenAIMessage>>, String> {
        let conn = self.connect()?;
        let record: Option<String> = conn
            .query_row(
                "SELECT messages_json FROM chat_history WHERE response_id = ?1",
                params![response_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| format!("load chat history failed: {err}"))?;

        let Some(messages_json) = record else {
            return Ok(None);
        };

        serde_json::from_str::<Vec<OpenAIMessage>>(&messages_json)
            .map(Some)
            .map_err(|err| format!("decode chat history messages failed: {err}"))
    }

    fn init(&self) -> Result<(), String> {
        if let Some(parent) = self.db_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("create chat history dir failed: {err}"))?;
        }

        let conn = self.connect()?;
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;

            CREATE TABLE IF NOT EXISTS chat_history (
                response_id TEXT PRIMARY KEY,
                messages_json TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_chat_history_updated_at
                ON chat_history (updated_at);
            ",
        )
        .map_err(|err| format!("initialize chat history schema failed: {err}"))?;

        Ok(())
    }

    fn connect(&self) -> Result<Connection, String> {
        Connection::open(&self.db_path).map_err(|err| {
            format!(
                "open chat history database {} failed: {err}",
                self.db_path.display()
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::ChatHistoryStore;
    use crate::models::{OpenAIContent, OpenAIMessage};
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn saves_and_loads_messages() {
        let db_path = unique_test_db_path("chat-history");
        let store = ChatHistoryStore::from_path(db_path.clone()).expect("create store");

        let messages = vec![OpenAIMessage {
            role: "user".to_string(),
            content: Some(OpenAIContent::String("hello".to_string())),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }];

        store
            .save_messages("resp_123", &messages)
            .expect("save messages");

        let loaded = store
            .load_messages("resp_123")
            .expect("load messages")
            .expect("messages should exist");

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].role, "user");

        let _ = fs::remove_file(db_path);
    }

    impl ChatHistoryStore {
        fn from_path(db_path: PathBuf) -> Result<Self, String> {
            let store = Self { db_path };
            store.init()?;
            Ok(store)
        }
    }

    fn unique_test_db_path(prefix: &str) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("current time")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{timestamp}.sqlite"))
    }
}
