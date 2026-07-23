use chrono::{Datelike, Days, Local, NaiveDate, TimeZone, Utc};
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path, sync::Arc};

const DATABASE_FILENAME: &str = "momentrace.sqlite3";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub executable: String,
    pub display_name: String,
    pub process_path: String,
    pub category: String,
    pub ignored: bool,
    #[serde(skip_serializing)]
    pub continue_tracking_while_idle: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveStatus {
    pub tracking: bool,
    pub paused: bool,
    pub idle_seconds: u64,
    pub current_app: Option<AppInfo>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetStatus {
    pub tracking: bool,
    pub paused: bool,
    pub idle_seconds: u64,
    pub current_app: Option<AppInfo>,
    pub current_app_seconds: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageItem {
    pub name: String,
    pub seconds: i64,
    pub color: Option<String>,
    pub executable: Option<String>,
    pub process_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DaySummary {
    pub date: String,
    pub total_seconds: i64,
    pub categories: Vec<UsageItem>,
    pub applications: Vec<UsageItem>,
    pub hours: Vec<UsageItem>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RangeSummary {
    pub days: Vec<UsageItem>,
    pub categories: Vec<UsageItem>,
    pub applications: Vec<UsageItem>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AllSummary {
    pub months: Vec<UsageItem>,
    pub categories: Vec<UsageItem>,
    pub applications: Vec<UsageItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    pub id: i64,
    pub name: String,
    pub color: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppRule {
    pub executable: String,
    pub display_name: String,
    pub process_path: String,
    pub category_id: Option<i64>,
    pub ignored: bool,
    #[serde(default)]
    pub continue_tracking_while_idle: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppRulePatch {
    pub display_name: Option<String>,
    pub category_id: Option<i64>,
    #[serde(default)]
    pub category_id_changed: bool,
    pub ignored: Option<bool>,
    pub continue_tracking_while_idle: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub idle_minutes: u32,
    pub launch_at_login: bool,
    pub show_widget: bool,
    pub font_family: String,
    pub widget_height: u32,
    pub widget_x_offset: i32,
    pub widget_y_offset: i32,
}

pub struct Store {
    connection: Mutex<Connection>,
}

impl Store {
    pub fn open(data_dir: &Path) -> Result<Arc<Self>, String> {
        fs::create_dir_all(data_dir).map_err(|err| err.to_string())?;
        let mut connection =
            Connection::open(data_dir.join(DATABASE_FILENAME)).map_err(|err| err.to_string())?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|err| err.to_string())?;
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS categories (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, color TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS app_rules (executable TEXT PRIMARY KEY, display_name TEXT NOT NULL, process_path TEXT NOT NULL, category_id INTEGER, ignored INTEGER NOT NULL DEFAULT 0, continue_tracking_while_idle INTEGER NOT NULL DEFAULT 0, FOREIGN KEY(category_id) REFERENCES categories(id) ON DELETE SET NULL);
             CREATE TABLE IF NOT EXISTS sessions (id INTEGER PRIMARY KEY, executable TEXT NOT NULL, display_name TEXT NOT NULL, process_path TEXT NOT NULL, started_at INTEGER NOT NULL, ended_at INTEGER NOT NULL, CHECK(ended_at >= started_at));
             CREATE INDEX IF NOT EXISTS sessions_time ON sessions(started_at, ended_at);
             CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);"
        ).map_err(|err| err.to_string())?;
        if !column_exists(&connection, "app_rules", "continue_tracking_while_idle")? {
            connection
                .execute(
                    "ALTER TABLE app_rules ADD COLUMN continue_tracking_while_idle INTEGER NOT NULL DEFAULT 0",
                    [],
                )
                .map_err(|err| err.to_string())?;
        }
        let categories_initialized: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM settings WHERE key='category_defaults_initialized')",
                [],
                |row| row.get(0),
            )
            .map_err(|err| err.to_string())?;
        if !categories_initialized {
            for (name, color) in [
                ("工作", "#6E8CFF"),
                ("学习", "#A77BFF"),
                ("沟通", "#46C2A6"),
                ("娱乐", "#FF8E72"),
                ("工具", "#E9BC5D"),
            ] {
                connection
                    .execute(
                        "INSERT OR IGNORE INTO categories(name, color) VALUES (?1, ?2)",
                        params![name, color],
                    )
                    .map_err(|err| err.to_string())?;
            }
            connection
                .execute(
                    "INSERT INTO settings(key, value) VALUES ('category_defaults_initialized', 'true')",
                    [],
                )
                .map_err(|err| err.to_string())?;
        }
        let reserved_category_id = connection
            .query_row(
                "SELECT id FROM categories WHERE name = '其他'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|err| err.to_string())?;
        if let Some(id) = reserved_category_id {
            let transaction = connection.transaction().map_err(|err| err.to_string())?;
            transaction
                .execute(
                    "UPDATE app_rules SET category_id = NULL WHERE category_id = ?1",
                    params![id],
                )
                .map_err(|err| err.to_string())?;
            transaction
                .execute("DELETE FROM categories WHERE id = ?1", params![id])
                .map_err(|err| err.to_string())?;
            transaction.commit().map_err(|err| err.to_string())?;
        }
        connection
            .execute(
                "INSERT OR IGNORE INTO settings(key, value) VALUES ('idle_minutes', '5')",
                [],
            )
            .map_err(|err| err.to_string())?;
        connection
            .execute(
                "INSERT OR IGNORE INTO settings(key, value) VALUES ('launch_at_login', 'false')",
                [],
            )
            .map_err(|err| err.to_string())?;
        connection
            .execute(
                "INSERT OR IGNORE INTO settings(key, value) VALUES ('show_widget', 'true')",
                [],
            )
            .map_err(|err| err.to_string())?;
        connection
            .execute(
                "INSERT OR IGNORE INTO settings(key, value) VALUES ('font_family', 'classic')",
                [],
            )
            .map_err(|err| err.to_string())?;
        for (key, value) in [
            ("widget_height", "64"),
            ("widget_x_offset", "0"),
            ("widget_y_offset", "0"),
        ] {
            connection
                .execute(
                    "INSERT OR IGNORE INTO settings(key, value) VALUES (?1, ?2)",
                    params![key, value],
                )
                .map_err(|err| err.to_string())?;
        }
        Ok(Arc::new(Self {
            connection: Mutex::new(connection),
        }))
    }

    pub fn rule_for(
        &self,
        executable: &str,
        display_name: &str,
        process_path: &str,
    ) -> Result<AppInfo, String> {
        let connection = self.connection.lock();
        connection.execute(
            "INSERT INTO app_rules(executable, display_name, process_path) VALUES (?1, ?2, ?3) ON CONFLICT(executable) DO UPDATE SET process_path=excluded.process_path",
            params![executable, display_name, process_path],
        ).map_err(|err| err.to_string())?;
        connection.query_row(
            "SELECT r.display_name, r.process_path, r.ignored, COALESCE(c.name, '其他'), r.continue_tracking_while_idle FROM app_rules r LEFT JOIN categories c ON c.id = r.category_id WHERE r.executable = ?1",
            params![executable], |row| Ok(AppInfo { executable: executable.into(), display_name: row.get(0)?, process_path: row.get(1)?, ignored: row.get::<_, i64>(2)? != 0, category: row.get(3)?, continue_tracking_while_idle: row.get::<_, i64>(4)? != 0 })
        ).map_err(|err| err.to_string())
    }

    pub fn append_session(
        &self,
        app: &AppInfo,
        started_at: i64,
        ended_at: i64,
    ) -> Result<(), String> {
        if app.ignored || ended_at <= started_at {
            return Ok(());
        }
        let connection = self.connection.lock();
        let previous: Option<(i64, i64)> = connection.query_row(
            "SELECT id, ended_at FROM sessions WHERE executable=?1 AND display_name=?2 ORDER BY ended_at DESC LIMIT 1", params![app.executable, app.display_name], |row| Ok((row.get(0)?, row.get(1)?))
        ).ok();
        if let Some((id, previous_end)) = previous {
            if previous_end == started_at {
                return connection
                    .execute(
                        "UPDATE sessions SET ended_at=?1 WHERE id=?2",
                        params![ended_at, id],
                    )
                    .map(|_| ())
                    .map_err(|err| err.to_string());
            }
        }
        connection.execute("INSERT INTO sessions(executable, display_name, process_path, started_at, ended_at) VALUES (?1, ?2, ?3, ?4, ?5)", params![app.executable, app.display_name, app.process_path, started_at, ended_at]).map(|_| ()).map_err(|err| err.to_string())
    }

    pub fn categories(&self) -> Result<Vec<Category>, String> {
        let connection = self.connection.lock();
        let mut statement = connection
            .prepare("SELECT id, name, color FROM categories ORDER BY id")
            .map_err(|err| err.to_string())?;
        let categories = statement
            .query_map([], |row| {
                Ok(Category {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    color: row.get(2)?,
                })
            })
            .map_err(|err| err.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;
        Ok(categories)
    }

    pub fn create_category(&self, name: &str, color: &str) -> Result<Category, String> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 20 {
            return Err("分类名称需要包含 1 至 20 个字符".into());
        }
        if name == "其他" {
            return Err("“其他”是未分类应用的保留名称".into());
        }
        let color = normalize_color(color)?;
        let connection = self.connection.lock();
        let exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM categories WHERE name = ?1 COLLATE NOCASE)",
                params![name],
                |row| row.get(0),
            )
            .map_err(|err| err.to_string())?;
        if exists {
            return Err("分类名称已存在".into());
        }
        connection
            .execute(
                "INSERT INTO categories(name, color) VALUES (?1, ?2)",
                params![name, color],
            )
            .map_err(|err| err.to_string())?;
        Ok(Category {
            id: connection.last_insert_rowid(),
            name: name.into(),
            color,
        })
    }

    pub fn delete_category(&self, id: i64) -> Result<(), String> {
        let mut connection = self.connection.lock();
        let transaction = connection.transaction().map_err(|err| err.to_string())?;
        let exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM categories WHERE id = ?1)",
                params![id],
                |row| row.get(0),
            )
            .map_err(|err| err.to_string())?;
        if !exists {
            return Err("分类不存在".into());
        }
        transaction
            .execute(
                "UPDATE app_rules SET category_id = NULL WHERE category_id = ?1",
                params![id],
            )
            .map_err(|err| err.to_string())?;
        transaction
            .execute("DELETE FROM categories WHERE id = ?1", params![id])
            .map_err(|err| err.to_string())?;
        transaction.commit().map_err(|err| err.to_string())
    }

    pub fn rules(&self) -> Result<Vec<AppRule>, String> {
        let connection = self.connection.lock();
        let mut statement = connection.prepare("SELECT executable, display_name, process_path, category_id, ignored, continue_tracking_while_idle FROM app_rules ORDER BY display_name COLLATE NOCASE").map_err(|err| err.to_string())?;
        let rules = statement
            .query_map([], |row| {
                Ok(AppRule {
                    executable: row.get(0)?,
                    display_name: row.get(1)?,
                    process_path: row.get(2)?,
                    category_id: row.get(3)?,
                    ignored: row.get::<_, i64>(4)? != 0,
                    continue_tracking_while_idle: row.get::<_, i64>(5)? != 0,
                })
            })
            .map_err(|err| err.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;
        Ok(rules)
    }

    pub fn icon_paths(&self, executable: &str) -> Result<Vec<String>, String> {
        let connection = self.connection.lock();
        let mut paths = Vec::new();
        let mut add_path = |path: String| {
            if !path.trim().is_empty() && !paths.iter().any(|value| value == &path) {
                paths.push(path);
            }
        };

        if let Some(path) = connection
            .query_row(
                "SELECT process_path FROM app_rules WHERE executable = ?1",
                params![executable],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|err| err.to_string())?
        {
            add_path(path);
        }

        let mut statement = connection
            .prepare(
                "SELECT process_path FROM sessions WHERE executable = ?1 ORDER BY ended_at DESC, id DESC",
            )
            .map_err(|err| err.to_string())?;
        let session_paths = statement
            .query_map(params![executable], |row| row.get::<_, String>(0))
            .map_err(|err| err.to_string())?;
        for path in session_paths {
            add_path(path.map_err(|err| err.to_string())?);
        }
        Ok(paths)
    }

    pub fn update_rule(&self, executable: &str, patch: &AppRulePatch) -> Result<(), String> {
        let connection = self.connection.lock();
        let current = connection
            .query_row(
                "SELECT display_name, category_id, ignored, continue_tracking_while_idle FROM app_rules WHERE executable = ?1",
                params![executable],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, i64>(2)? != 0,
                        row.get::<_, i64>(3)? != 0,
                    ))
                },
            )
            .map_err(|err| err.to_string())?;
        let display_name = patch.display_name.as_deref().unwrap_or(&current.0).trim();
        if display_name.is_empty() || display_name.chars().count() > 48 {
            return Err("应用名称需要包含 1 至 48 个字符".into());
        }
        let category_id = if patch.category_id_changed {
            patch.category_id
        } else {
            current.1
        };
        let ignored = patch.ignored.unwrap_or(current.2);
        let continue_tracking_while_idle = patch.continue_tracking_while_idle.unwrap_or(current.3);
        connection
            .execute(
                "UPDATE app_rules SET display_name=?1, category_id=?2, ignored=?3, continue_tracking_while_idle=?4 WHERE executable=?5",
                params![display_name, category_id, ignored as i64, continue_tracking_while_idle as i64, executable],
            )
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    pub fn settings(&self) -> Result<Settings, String> {
        let connection = self.connection.lock();
        let idle: String = connection
            .query_row(
                "SELECT value FROM settings WHERE key='idle_minutes'",
                [],
                |row| row.get(0),
            )
            .map_err(|err| err.to_string())?;
        let autostart: String = connection
            .query_row(
                "SELECT value FROM settings WHERE key='launch_at_login'",
                [],
                |row| row.get(0),
            )
            .map_err(|err| err.to_string())?;
        let show_widget: String = connection
            .query_row(
                "SELECT value FROM settings WHERE key='show_widget'",
                [],
                |row| row.get(0),
            )
            .map_err(|err| err.to_string())?;
        let font_family: String = connection
            .query_row(
                "SELECT value FROM settings WHERE key='font_family'",
                [],
                |row| row.get(0),
            )
            .map_err(|err| err.to_string())?;
        let widget_height: String = connection
            .query_row(
                "SELECT value FROM settings WHERE key='widget_height'",
                [],
                |row| row.get(0),
            )
            .map_err(|err| err.to_string())?;
        let widget_x_offset: String = connection
            .query_row(
                "SELECT value FROM settings WHERE key='widget_x_offset'",
                [],
                |row| row.get(0),
            )
            .map_err(|err| err.to_string())?;
        let widget_y_offset: String = connection
            .query_row(
                "SELECT value FROM settings WHERE key='widget_y_offset'",
                [],
                |row| row.get(0),
            )
            .map_err(|err| err.to_string())?;
        Ok(Settings {
            idle_minutes: idle.parse().unwrap_or(5),
            launch_at_login: autostart == "true",
            show_widget: show_widget == "true",
            font_family: normalize_font_family(&font_family).into(),
            widget_height: widget_height.parse().unwrap_or(64).clamp(48, 160),
            widget_x_offset: widget_x_offset.parse().unwrap_or(0).clamp(-1000, 1000),
            widget_y_offset: widget_y_offset.parse().unwrap_or(0).clamp(-1000, 1000),
        })
    }

    pub fn update_settings(&self, settings: &Settings) -> Result<(), String> {
        let connection = self.connection.lock();
        connection
            .execute(
                "UPDATE settings SET value=?1 WHERE key='idle_minutes'",
                params![settings.idle_minutes.clamp(1, 120).to_string()],
            )
            .map_err(|err| err.to_string())?;
        connection
            .execute(
                "UPDATE settings SET value=?1 WHERE key='launch_at_login'",
                params![settings.launch_at_login.to_string()],
            )
            .map_err(|err| err.to_string())?;
        connection
            .execute(
                "UPDATE settings SET value=?1 WHERE key='show_widget'",
                params![settings.show_widget.to_string()],
            )
            .map_err(|err| err.to_string())?;
        connection
            .execute(
                "UPDATE settings SET value=?1 WHERE key='font_family'",
                params![normalize_font_family(&settings.font_family)],
            )
            .map_err(|err| err.to_string())?;
        for (key, value) in [
            (
                "widget_height",
                settings.widget_height.clamp(48, 160).to_string(),
            ),
            (
                "widget_x_offset",
                settings.widget_x_offset.clamp(-1000, 1000).to_string(),
            ),
            (
                "widget_y_offset",
                settings.widget_y_offset.clamp(-1000, 1000).to_string(),
            ),
        ] {
            connection
                .execute(
                    "UPDATE settings SET value=?1 WHERE key=?2",
                    params![value, key],
                )
                .map_err(|err| err.to_string())?;
        }
        Ok(())
    }

    fn range(date: &str) -> Result<(i64, i64), String> {
        let date = NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|err| err.to_string())?;
        Self::date_range(date)
    }

    fn date_range(date: NaiveDate) -> Result<(i64, i64), String> {
        let next = date
            .checked_add_days(Days::new(1))
            .ok_or("Date is out of range")?;
        let start = date
            .and_hms_opt(0, 0, 0)
            .and_then(|value| Local.from_local_datetime(&value).single())
            .ok_or("Unable to determine local day start")?;
        let end = next
            .and_hms_opt(0, 0, 0)
            .and_then(|value| Local.from_local_datetime(&value).single())
            .ok_or("Unable to determine local day end")?;
        Ok((
            start.with_timezone(&Utc).timestamp_millis(),
            end.with_timezone(&Utc).timestamp_millis(),
        ))
    }

    fn bucket_usage(
        &self,
        dates: impl IntoIterator<Item = NaiveDate>,
        label: &str,
    ) -> Result<Vec<UsageItem>, String> {
        let connection = self.connection.lock();
        let mut statement = connection.prepare(
            "SELECT COALESCE(SUM(MAX(0, MIN(ended_at, ?2) - MAX(started_at, ?1)) / 1000), 0) FROM sessions WHERE ended_at>?1 AND started_at<?2"
        ).map_err(|err| err.to_string())?;
        let mut items = Vec::new();
        for date in dates {
            let (start, end) = Self::date_range(date)?;
            let seconds: i64 = statement
                .query_row(params![start, end], |row| row.get(0))
                .map_err(|err| err.to_string())?;
            if seconds > 0 {
                items.push(UsageItem {
                    name: date.format(label).to_string(),
                    seconds,
                    color: None,
                    executable: None,
                    process_path: None,
                });
            }
        }
        Ok(items)
    }

    fn hour_usage(&self, date: NaiveDate) -> Result<Vec<UsageItem>, String> {
        let connection = self.connection.lock();
        let mut statement = connection.prepare(
            "SELECT COALESCE(SUM(MAX(0, MIN(ended_at, ?2) - MAX(started_at, ?1)) / 1000), 0) FROM sessions WHERE ended_at>?1 AND started_at<?2"
        ).map_err(|err| err.to_string())?;
        let (day_start, day_end) = Self::date_range(date)?;
        let mut by_hour = std::collections::BTreeMap::<String, i64>::new();
        let mut start = day_start;
        while start < day_end {
            let end = start.saturating_add(3_600_000).min(day_end);
            let seconds: i64 = statement
                .query_row(params![start, end], |row| row.get(0))
                .map_err(|err| err.to_string())?;
            if seconds > 0 {
                let name = Local
                    .timestamp_millis_opt(start)
                    .single()
                    .ok_or("Unable to determine local hour")?
                    .format("%H:00")
                    .to_string();
                *by_hour.entry(name).or_default() += seconds;
            }
            start = end;
        }
        Ok(by_hour
            .into_iter()
            .map(|(name, seconds)| UsageItem {
                name,
                seconds,
                color: None,
                executable: None,
                process_path: None,
            })
            .collect())
    }

    fn all_month_usage(&self) -> Result<Vec<UsageItem>, String> {
        let connection = self.connection.lock();
        let (first, last): (Option<i64>, Option<i64>) = connection
            .query_row(
                "SELECT MIN(started_at), MAX(ended_at) FROM sessions",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|err| err.to_string())?;
        let (Some(first), Some(last)) = (first, last) else {
            return Ok(Vec::new());
        };
        let first_date = Local
            .timestamp_millis_opt(first)
            .single()
            .ok_or("Unable to determine first recorded month")?
            .date_naive();
        let mut month = NaiveDate::from_ymd_opt(first_date.year(), first_date.month(), 1)
            .ok_or("Unable to determine first recorded month")?;
        let mut statement = connection.prepare(
            "SELECT COALESCE(SUM(MAX(0, MIN(ended_at, ?2) - MAX(started_at, ?1)) / 1000), 0) FROM sessions WHERE ended_at>?1 AND started_at<?2",
        ).map_err(|err| err.to_string())?;
        let mut items = Vec::new();
        loop {
            let next_month = if month.month() == 12 {
                NaiveDate::from_ymd_opt(month.year() + 1, 1, 1)
            } else {
                NaiveDate::from_ymd_opt(month.year(), month.month() + 1, 1)
            }
            .ok_or("Unable to determine next recorded month")?;
            let start = Self::date_range(month)?.0;
            if start >= last {
                break;
            }
            let end = Self::date_range(next_month)?.0;
            let seconds: i64 = statement
                .query_row(params![start, end], |row| row.get(0))
                .map_err(|err| err.to_string())?;
            if seconds > 0 {
                items.push(UsageItem {
                    name: month.format("%Y/%m").to_string(),
                    seconds,
                    color: None,
                    executable: None,
                    process_path: None,
                });
            }
            month = next_month;
        }
        Ok(items)
    }

    fn grouped(
        &self,
        sql: &str,
        values: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<UsageItem>, String> {
        let connection = self.connection.lock();
        let mut statement = connection.prepare(sql).map_err(|err| err.to_string())?;
        let items = statement
            .query_map(values, |row| {
                Ok(UsageItem {
                    name: row.get(0)?,
                    seconds: row.get(1)?,
                    color: row.get(2)?,
                    executable: None,
                    process_path: None,
                })
            })
            .map_err(|err| err.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;
        Ok(items)
    }

    fn application_usage(
        &self,
        sql: &str,
        values: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<UsageItem>, String> {
        let connection = self.connection.lock();
        let mut statement = connection.prepare(sql).map_err(|err| err.to_string())?;
        let items = statement
            .query_map(values, |row| {
                Ok(UsageItem {
                    name: row.get(0)?,
                    seconds: row.get(1)?,
                    color: None,
                    executable: row.get(2)?,
                    process_path: row.get(3)?,
                })
            })
            .map_err(|err| err.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;
        Ok(items)
    }

    pub fn today_app_seconds(&self, executable: &str) -> Result<i64, String> {
        let (start, end) = Self::range(&today())?;
        self.connection.lock().query_row(
            "SELECT COALESCE(SUM(MAX(0, MIN(ended_at, ?3) - MAX(started_at, ?2)) / 1000), 0) FROM sessions WHERE executable=?1 AND ended_at>?2 AND started_at<?3",
            params![executable, start, end],
            |row| row.get(0),
        ).map_err(|err| err.to_string())
    }
    pub fn day_summary(&self, date: &str) -> Result<DaySummary, String> {
        let (start, end) = Self::range(date)?;
        let total: i64 = self.connection.lock().query_row("SELECT COALESCE(SUM(MAX(0, MIN(ended_at, ?2) - MAX(started_at, ?1)) / 1000), 0) FROM sessions WHERE ended_at>?1 AND started_at<?2", params![start, end], |row| row.get(0)).map_err(|err| err.to_string())?;
        let categories = self.grouped("SELECT COALESCE(c.name, '其他'), COALESCE(SUM(MAX(0, MIN(s.ended_at, ?2) - MAX(s.started_at, ?1)) / 1000),0), COALESCE(c.color, '#8391A5') FROM sessions s LEFT JOIN app_rules r ON r.executable=s.executable LEFT JOIN categories c ON c.id=r.category_id WHERE s.ended_at>?1 AND s.started_at<?2 GROUP BY c.name, c.color ORDER BY 2 DESC, 1 COLLATE NOCASE", &[&start, &end])?;
        let applications = self.application_usage("SELECT COALESCE(r.display_name, s.display_name), COALESCE(SUM(MAX(0, MIN(s.ended_at, ?2) - MAX(s.started_at, ?1)) / 1000),0), s.executable, COALESCE(r.process_path, MAX(s.process_path)) FROM sessions s LEFT JOIN app_rules r ON r.executable=s.executable WHERE s.ended_at>?1 AND s.started_at<?2 GROUP BY s.executable, r.display_name ORDER BY 2 DESC, 1 COLLATE NOCASE, s.executable COLLATE NOCASE", &[&start, &end])?;
        let parsed_date =
            NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|err| err.to_string())?;
        let hours = self.hour_usage(parsed_date)?;
        Ok(DaySummary {
            date: date.into(),
            total_seconds: total,
            categories,
            applications,
            hours,
        })
    }

    pub fn range_summary(&self, start_date: &str, end_date: &str) -> Result<RangeSummary, String> {
        let first =
            NaiveDate::parse_from_str(start_date, "%Y-%m-%d").map_err(|err| err.to_string())?;
        let last =
            NaiveDate::parse_from_str(end_date, "%Y-%m-%d").map_err(|err| err.to_string())?;
        if last < first {
            return Err("End date must not be before start date".into());
        }
        let day_count = (last - first).num_days() + 1;
        if day_count > 3_660 {
            return Err("Date range must not exceed 10 years".into());
        }
        let end_exclusive = last
            .checked_add_days(Days::new(1))
            .ok_or("Date is out of range")?;
        let start = Self::date_range(first)?.0;
        let end = Self::date_range(end_exclusive)?.0;
        let label = if first.year() == last.year() {
            "%m/%d"
        } else {
            "%Y/%m/%d"
        };
        let days = self.bucket_usage(
            std::iter::successors(Some(first), |day| day.checked_add_days(Days::new(1)))
                .take_while(|day| *day <= last),
            label,
        )?;
        let categories = self.grouped("SELECT COALESCE(c.name, '其他'), SUM(MAX(0, MIN(s.ended_at, ?2) - MAX(s.started_at, ?1)) / 1000), COALESCE(c.color, '#8391A5') FROM sessions s LEFT JOIN app_rules r ON r.executable=s.executable LEFT JOIN categories c ON c.id=r.category_id WHERE s.ended_at>?1 AND s.started_at<?2 GROUP BY c.name,c.color ORDER BY 2 DESC, 1 COLLATE NOCASE", &[&start, &end])?;
        let applications = self.application_usage("SELECT COALESCE(r.display_name, s.display_name), SUM(MAX(0, MIN(s.ended_at, ?2) - MAX(s.started_at, ?1)) / 1000), s.executable, COALESCE(r.process_path, MAX(s.process_path)) FROM sessions s LEFT JOIN app_rules r ON r.executable=s.executable WHERE s.ended_at>?1 AND s.started_at<?2 GROUP BY s.executable, r.display_name ORDER BY 2 DESC, 1 COLLATE NOCASE, s.executable COLLATE NOCASE", &[&start, &end])?;
        Ok(RangeSummary {
            days,
            categories,
            applications,
        })
    }

    pub fn all_summary(&self) -> Result<AllSummary, String> {
        let months = self.all_month_usage()?;
        let categories = self.grouped("SELECT COALESCE(c.name, '其他'), SUM((s.ended_at-s.started_at)/1000), COALESCE(c.color, '#8391A5') FROM sessions s LEFT JOIN app_rules r ON r.executable=s.executable LEFT JOIN categories c ON c.id=r.category_id GROUP BY c.name,c.color ORDER BY 2 DESC, 1 COLLATE NOCASE", &[])?;
        let applications = self.application_usage("SELECT COALESCE(r.display_name, s.display_name), SUM((s.ended_at-s.started_at)/1000), s.executable, COALESCE(r.process_path, MAX(s.process_path)) FROM sessions s LEFT JOIN app_rules r ON r.executable=s.executable GROUP BY s.executable, r.display_name ORDER BY 2 DESC, 1 COLLATE NOCASE, s.executable COLLATE NOCASE", &[])?;
        Ok(AllSummary {
            months,
            categories,
            applications,
        })
    }

    pub fn clear(&self) -> Result<(), String> {
        self.connection
            .lock()
            .execute("DELETE FROM sessions", [])
            .map(|_| ())
            .map_err(|err| err.to_string())
    }
}

pub fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}
pub fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

fn normalize_font_family(value: &str) -> &'static str {
    match value {
        "serif" => "serif",
        "sans" => "sans",
        _ => "classic",
    }
}

fn normalize_color(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.len() == 7
        && value.starts_with('#')
        && value[1..]
            .bytes()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Ok(value.to_ascii_uppercase());
    }
    Err("分类颜色必须是 #RRGGBB 格式".into())
}

fn column_exists(connection: &Connection, table: &str, column: &str) -> Result<bool, String> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|err| err.to_string())?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|err| err.to_string())?;
    for name in columns {
        if name.map_err(|err| err.to_string())? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> (tempfile::TempDir, Arc<Store>) {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path()).unwrap();
        (directory, store)
    }

    fn local_ms(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> i64 {
        Local
            .with_ymd_and_hms(year, month, day, hour, minute, second)
            .single()
            .unwrap()
            .with_timezone(&Utc)
            .timestamp_millis()
    }

    fn insert_session(store: &Store, started_at: i64, ended_at: i64) {
        store.connection.lock().execute(
            "INSERT INTO sessions(executable, display_name, process_path, started_at, ended_at) VALUES ('test.exe', 'Test', 'C:\\test.exe', ?1, ?2)",
            params![started_at, ended_at],
        ).unwrap();
    }

    fn insert_app_session(store: &Store, index: usize, started_at: i64, ended_at: i64) {
        let executable = format!("app-{index}.exe");
        let display_name = format!("App {index}");
        let process_path = format!("C:\\{executable}");
        store
            .connection
            .lock()
            .execute(
                "INSERT INTO sessions(executable, display_name, process_path, started_at, ended_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![executable, display_name, process_path, started_at, ended_at],
            )
            .unwrap();
    }

    #[test]
    fn font_family_setting_is_persisted_and_validated() {
        let (_directory, store) = test_store();
        let mut settings = store.settings().unwrap();
        assert_eq!(settings.font_family, "classic");

        settings.font_family = "serif".into();
        store.update_settings(&settings).unwrap();
        assert_eq!(store.settings().unwrap().font_family, "serif");

        settings.font_family = "untrusted-value".into();
        settings.widget_height = 999;
        settings.widget_x_offset = -5000;
        settings.widget_y_offset = 5000;
        store.update_settings(&settings).unwrap();
        let settings = store.settings().unwrap();
        assert_eq!(settings.font_family, "classic");
        assert_eq!(settings.widget_height, 160);
        assert_eq!(settings.widget_x_offset, -1000);
        assert_eq!(settings.widget_y_offset, 1000);
    }

    #[test]
    fn day_summary_splits_sessions_across_hours() {
        let (_directory, store) = test_store();
        insert_session(
            &store,
            local_ms(2026, 7, 13, 10, 59, 0),
            local_ms(2026, 7, 13, 11, 1, 0),
        );

        let summary = store.day_summary("2026-07-13").unwrap();

        assert_eq!(summary.total_seconds, 120);
        assert_eq!(
            summary
                .hours
                .iter()
                .map(|item| (item.name.as_str(), item.seconds))
                .collect::<Vec<_>>(),
            [("10:00", 60), ("11:00", 60)]
        );
    }

    #[test]
    fn application_summaries_are_not_truncated() {
        let (_directory, store) = test_store();
        let start = local_ms(2026, 7, 13, 12, 0, 0);
        for index in 0..12 {
            let app_start = start + index as i64 * 2_000;
            insert_app_session(&store, index, app_start, app_start + 1_000);
        }

        assert_eq!(
            store.day_summary("2026-07-13").unwrap().applications.len(),
            12
        );
        assert_eq!(
            store
                .range_summary("2026-07-13", "2026-07-13")
                .unwrap()
                .applications
                .len(),
            12
        );
        assert_eq!(store.all_summary().unwrap().applications.len(), 12);
    }

    #[test]
    fn all_summary_splits_sessions_across_months() {
        let (_directory, store) = test_store();
        insert_session(
            &store,
            local_ms(2026, 6, 30, 23, 59, 30),
            local_ms(2026, 7, 1, 0, 0, 30),
        );

        let summary = store.all_summary().unwrap();

        assert_eq!(
            summary
                .months
                .iter()
                .map(|item| (item.name.as_str(), item.seconds))
                .collect::<Vec<_>>(),
            [("2026/06", 30), ("2026/07", 30)]
        );
        assert_eq!(summary.applications[0].seconds, 60);
    }

    #[test]
    fn range_summary_includes_both_dates_and_clips_boundaries() {
        let (_directory, store) = test_store();
        insert_session(
            &store,
            local_ms(2026, 7, 12, 23, 59, 30),
            local_ms(2026, 7, 13, 0, 0, 30),
        );
        insert_session(
            &store,
            local_ms(2026, 7, 14, 23, 59, 30),
            local_ms(2026, 7, 15, 0, 0, 30),
        );

        let summary = store.range_summary("2026-07-13", "2026-07-14").unwrap();

        assert_eq!(
            summary
                .days
                .iter()
                .map(|item| (item.name.as_str(), item.seconds))
                .collect::<Vec<_>>(),
            [("07/13", 30), ("07/14", 30)]
        );
        assert_eq!(summary.applications[0].seconds, 60);
        assert!(store.range_summary("2026-07-15", "2026-07-14").is_err());
    }

    #[test]
    fn rule_for_refreshes_path_without_losing_custom_name() {
        let (_directory, store) = test_store();
        store
            .rule_for("test.exe", "Test", "C:\\old\\test.exe")
            .unwrap();
        store
            .update_rule(
                "test.exe",
                &AppRulePatch {
                    display_name: Some("Custom".into()),
                    category_id: None,
                    category_id_changed: false,
                    ignored: None,
                    continue_tracking_while_idle: Some(true),
                },
            )
            .unwrap();

        let rule = store
            .rule_for("test.exe", "Test", "D:\\new\\test.exe")
            .unwrap();

        assert_eq!(rule.display_name, "Custom");
        assert_eq!(rule.process_path, "D:\\new\\test.exe");
        assert!(rule.continue_tracking_while_idle);
        store
            .update_rule(
                "test.exe",
                &AppRulePatch {
                    display_name: None,
                    category_id: None,
                    category_id_changed: false,
                    ignored: Some(true),
                    continue_tracking_while_idle: None,
                },
            )
            .unwrap();
        let saved = &store.rules().unwrap()[0];
        assert_eq!(saved.display_name, "Custom");
        assert!(saved.ignored);
        assert!(saved.continue_tracking_while_idle);
    }

    #[test]
    fn icon_paths_prefer_the_current_rule_path_and_keep_recent_fallbacks() {
        let (_directory, store) = test_store();
        let start = local_ms(2026, 7, 13, 12, 0, 0);
        store
            .rule_for("icon.exe", "Icon", "D:\\current\\icon.exe")
            .unwrap();
        {
            let connection = store.connection.lock();
            connection
                .execute(
                    "INSERT INTO sessions(executable, display_name, process_path, started_at, ended_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params!["icon.exe", "Icon", "C:\\old\\icon.exe", start, start + 1_000],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO sessions(executable, display_name, process_path, started_at, ended_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params!["icon.exe", "Icon", "E:\\recent\\icon.exe", start + 2_000, start + 3_000],
                )
                .unwrap();
        }

        assert_eq!(
            store.icon_paths("icon.exe").unwrap(),
            [
                "D:\\current\\icon.exe",
                "E:\\recent\\icon.exe",
                "C:\\old\\icon.exe",
            ]
        );
        let applications = store.all_summary().unwrap().applications;
        assert_eq!(
            applications[0].process_path.as_deref(),
            Some("D:\\current\\icon.exe")
        );
    }

    #[test]
    fn open_adds_idle_tracking_policy_to_existing_rule_schema() {
        let directory = tempfile::tempdir().unwrap();
        let connection = Connection::open(directory.path().join(DATABASE_FILENAME)).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE app_rules (
                    executable TEXT PRIMARY KEY,
                    display_name TEXT NOT NULL,
                    process_path TEXT NOT NULL,
                    category_id INTEGER,
                    ignored INTEGER NOT NULL DEFAULT 0
                );",
            )
            .unwrap();
        drop(connection);

        let store = Store::open(directory.path()).unwrap();
        let rule = store.rule_for("test.exe", "Test", "C:\\test.exe").unwrap();

        assert!(!rule.continue_tracking_while_idle);
    }

    #[test]
    fn categories_can_be_created_and_deleted_without_returning() {
        let (directory, store) = test_store();
        let category = store.create_category("影音", "#12abEF").unwrap();
        assert_eq!(category.color, "#12ABEF");
        assert!(store.create_category("影音", "#000000").is_err());
        assert!(store.create_category("无效颜色", "blue").is_err());
        assert!(store.create_category("其他", "#8391A5").is_err());

        store
            .rule_for("video.exe", "Video", "C:\\video.exe")
            .unwrap();
        store
            .update_rule(
                "video.exe",
                &AppRulePatch {
                    display_name: None,
                    category_id: Some(category.id),
                    category_id_changed: true,
                    ignored: None,
                    continue_tracking_while_idle: Some(true),
                },
            )
            .unwrap();
        store.delete_category(category.id).unwrap();
        let rule = &store.rules().unwrap()[0];
        assert_eq!(rule.category_id, None);
        assert!(rule.continue_tracking_while_idle);

        let default_id = store
            .categories()
            .unwrap()
            .into_iter()
            .find(|value| value.name == "工作")
            .unwrap()
            .id;
        store.delete_category(default_id).unwrap();
        {
            let connection = store.connection.lock();
            connection
                .execute(
                    "INSERT INTO categories(name, color) VALUES ('其他', '#8391A5')",
                    [],
                )
                .unwrap();
            let reserved_id = connection.last_insert_rowid();
            connection
                .execute(
                    "UPDATE app_rules SET category_id = ?1 WHERE executable = 'video.exe'",
                    params![reserved_id],
                )
                .unwrap();
            connection
                .execute_batch(
                    "INSERT INTO sessions(executable, display_name, process_path, started_at, ended_at)
                     VALUES ('video.exe', 'Video', 'C:\\video.exe', 1000, 2000);
                     INSERT INTO sessions(executable, display_name, process_path, started_at, ended_at)
                     VALUES ('other.exe', 'Other', 'C:\\other.exe', 2000, 3000);",
                )
                .unwrap();
        }
        drop(store);

        let reopened = Store::open(directory.path()).unwrap();
        assert!(!reopened
            .categories()
            .unwrap()
            .iter()
            .any(|value| value.name == "工作"));
        assert!(!reopened
            .categories()
            .unwrap()
            .iter()
            .any(|value| value.name == "其他"));
        assert_eq!(reopened.rules().unwrap()[0].category_id, None);
        let summary = reopened.all_summary().unwrap();
        assert_eq!(summary.categories.len(), 1);
        assert_eq!(summary.categories[0].name, "其他");
        assert_eq!(summary.categories[0].seconds, 2);
    }

    #[test]
    fn append_session_does_not_merge_across_another_application() {
        let (_directory, store) = test_store();
        let first = AppInfo {
            executable: "a.exe".into(),
            display_name: "A".into(),
            process_path: "C:\\a.exe".into(),
            category: "其他".into(),
            ignored: false,
            continue_tracking_while_idle: false,
        };
        let other = AppInfo {
            executable: "b.exe".into(),
            display_name: "B".into(),
            process_path: "C:\\b.exe".into(),
            category: "其他".into(),
            ignored: false,
            continue_tracking_while_idle: false,
        };
        store.append_session(&first, 1_000, 2_000).unwrap();
        store.append_session(&other, 2_000, 2_500).unwrap();
        store.append_session(&first, 2_500, 3_000).unwrap();

        let count: i64 = store
            .connection
            .lock()
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 3);
    }
}
