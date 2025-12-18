use tauri_plugin_sql::{Migration, MigrationKind};

/// 获取数据库迁移配置
pub fn get_migrations() -> Vec<Migration> {
    vec![
        Migration {
            version: 1,
            description: "create_agents_table",
            sql: r#"
                CREATE TABLE IF NOT EXISTS agents (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL UNIQUE,
                    system_prompt TEXT DEFAULT '',
                    allow_tools INTEGER DEFAULT 1,
                    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
                )
            "#,
            kind: MigrationKind::Up,
        },
        Migration {
            version: 2,
            description: "create_config_table",
            sql: r#"
                CREATE TABLE IF NOT EXISTS config (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    key TEXT NOT NULL UNIQUE,
                    value TEXT DEFAULT '{}',
                    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
                )
            "#,
            kind: MigrationKind::Up,
        },
    ]
}
