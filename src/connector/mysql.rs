use crate::{
    ast::{Id, ParameterizedValue, Query},
    error::Error,
    transaction::{
        ColumnNames, Connection, Connectional, ResultRow, ToColumnNames, ToResultRow, Transaction,
        Transactional,
    },
    visitor::{self, Visitor},
    QueryResult, ResultSet,
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use mysql as my;
use r2d2_mysql::pool::MysqlConnectionManager;
use url::Url;

type Pool = r2d2::Pool<MysqlConnectionManager>;
#[allow(unused)] // We implement a trait on the alias, it is used.
type PooledConnection = r2d2::PooledConnection<MysqlConnectionManager>;

/// The World's Most Advanced Open Source Relational Database
pub struct Mysql {
    pool: Pool,
    pub db_name: Option<String>,
}

impl Mysql {
    // TODO: we should not use this constructor since it does set the db_name field
    pub fn new(conf: mysql::OptsBuilder) -> QueryResult<Mysql> {
        let manager = MysqlConnectionManager::new(conf);

        Ok(Mysql {
            pool: r2d2::Pool::builder().build(manager)?,
            db_name: None,
        })
    }

    pub fn new_from_url(url: &str) -> QueryResult<Mysql> {
        // TODO: connection limit configuration
        let mut builder = my::OptsBuilder::new();
        let url = Url::parse(url)?;
        let db_name = url.path_segments().and_then(|mut segments| segments.next());

        builder.ip_or_hostname(url.host_str());
        builder.tcp_port(url.port().unwrap_or(3306));
        builder.user(Some(url.username()));
        builder.pass(url.password());
        builder.db_name(db_name);
        builder.verify_peer(false);
        builder.stmt_cache_size(Some(1000));

        let manager = MysqlConnectionManager::new(builder);

        Ok(Mysql {
            pool: r2d2::Pool::builder().build(manager)?,
            db_name: db_name.map(|x| x.to_string()),
        })
    }
}

impl Transactional for Mysql {
    fn with_transaction<F, T>(&self, _db: &str, f: F) -> QueryResult<T>
    where
        F: FnOnce(&mut Transaction) -> QueryResult<T>,
    {
        let mut conn = self.pool.get()?;
        let mut tx = conn.start_transaction(true, None, None)?;
        let result = f(&mut tx);

        if result.is_ok() {
            tx.commit()?;
        }

        result
    }
}

impl Connectional for Mysql {
    fn with_connection<F, T>(&self, _db: &str, f: F) -> QueryResult<T>
    where
        F: FnOnce(&mut Connection) -> QueryResult<T>,
        Self: Sized,
    {
        dbg!(self.pool.state());
        let mut conn = self.pool.get()?;
        let result = f(&mut conn);
        result
    }

    fn execute_on_connection(&self, db: &str, query: Query) -> QueryResult<Option<Id>> {
        self.with_connection(&db, |conn| conn.execute(query))
    }

    fn query_on_connection(&self, db: &str, query: Query) -> QueryResult<ResultSet> {
        self.with_connection(&db, |conn| conn.query(query))
    }

    fn query_on_raw_connection(
        &self,
        db: &str,
        sql: &str,
        params: &[ParameterizedValue],
    ) -> QueryResult<ResultSet> {
        self.with_connection(&db, |conn| conn.query_raw(&sql, &params))
    }
}

fn conv_params(params: &[ParameterizedValue]) -> my::params::Params {
    if params.len() > 0 {
        my::params::Params::Positional(params.iter().map(|x| x.into()).collect::<Vec<my::Value>>())
    } else {
        // If we don't use explicit 'Empty',
        // mysql crashes with 'internal error: entered unreachable code'
        my::params::Params::Empty
    }
}

impl<'a> Transaction for my::Transaction<'a> {}

impl<'a> Connection for my::Transaction<'a> {
    fn execute(&mut self, q: Query) -> QueryResult<Option<Id>> {
        let (sql, params) = dbg!(visitor::Mysql::build(q));
        let mut stmt = self.prepare(&sql)?;
        let _rows = stmt.execute(conv_params(&params))?;

        // TODO: Return last inserted ID is not implemented for mysql.
        Ok(None)
    }

    fn query(&mut self, q: Query) -> QueryResult<ResultSet> {
        let (sql, params) = dbg!(visitor::Mysql::build(q));

        self.query_raw(&sql, &params[..])
    }

    fn query_raw(&mut self, sql: &str, params: &[ParameterizedValue]) -> QueryResult<ResultSet> {
        let mut stmt = self.prepare(&sql)?;
        let mut result = ResultSet::new(&stmt.to_column_names(), Vec::new());
        let rows = stmt.execute(conv_params(params))?;

        for row in rows {
            result.rows.push(row?.to_result_row()?);
        }

        Ok(result)
    }
}

impl Connection for PooledConnection {
    fn execute(&mut self, q: Query) -> QueryResult<Option<Id>> {
        let (sql, params) = dbg!(visitor::Mysql::build(q));
        let mut stmt = self.prepare(&sql)?;
        let _rows = stmt.execute(conv_params(&params))?;

        Ok(Some(Id::Int(_rows.last_insert_id() as usize)))
    }

    fn query(&mut self, q: Query) -> QueryResult<ResultSet> {
        let (sql, params) = dbg!(visitor::Mysql::build(q));

        self.query_raw(&sql, &params[..])
    }

    fn query_raw(&mut self, sql: &str, params: &[ParameterizedValue]) -> QueryResult<ResultSet> {
        let mut stmt = self.prepare(&sql)?;
        let mut result = ResultSet::new(&stmt.to_column_names(), Vec::new());
        let rows = stmt.execute(conv_params(params))?;

        for row in rows {
            result.rows.push(row?.to_result_row()?);
        }

        Ok(result)
    }
}

impl ToResultRow for my::Row {
    fn to_result_row<'b>(&'b self) -> QueryResult<ResultRow> {
        fn convert(row: &my::Row, i: usize) -> QueryResult<ParameterizedValue> {
            // TODO: It would prob. be better to inver via Column::column_type()
            let raw_value = row.as_ref(i).unwrap_or(&my::Value::NULL);
            let res = match raw_value {
                my::Value::NULL => ParameterizedValue::Null,
                my::Value::Bytes(b) => ParameterizedValue::Text(String::from_utf8(b.to_vec())?),
                my::Value::Int(i) => ParameterizedValue::Integer(*i),
                // TOOD: This is unsafe
                my::Value::UInt(i) => ParameterizedValue::Integer(*i as i64),
                my::Value::Float(f) => ParameterizedValue::Real(*f),
                my::Value::Date(year, month, day, hour, min, sec, _) => {
                    let naive = NaiveDate::from_ymd(*year as i32, *month as u32, *day as u32)
                        .and_hms(*hour as u32, *min as u32, *sec as u32);

                    let dt: DateTime<Utc> = DateTime::from_utc(naive, Utc);
                    ParameterizedValue::DateTime(dt)
                }
                my::Value::Time(is_neg, days, hours, minutes, seconds, micros) => {
                    let days = Duration::days(*days as i64);
                    let hours = Duration::hours(*hours as i64);
                    let minutes = Duration::minutes(*minutes as i64);
                    let seconds = Duration::seconds(*seconds as i64);
                    let micros = Duration::microseconds(*micros as i64);

                    let time = days
                        .checked_add(&hours)
                        .and_then(|t| t.checked_add(&minutes))
                        .and_then(|t| t.checked_add(&seconds))
                        .and_then(|t| t.checked_add(&micros))
                        .unwrap();

                    let duration = time.to_std().unwrap();
                    let f_time = duration.as_secs() as f64 + duration.subsec_micros() as f64 * 1e-6;

                    ParameterizedValue::Real(if *is_neg { -f_time } else { f_time })
                }
            };

            Ok(res)
        }

        let mut row = ResultRow::default();

        for i in 0..self.len() {
            row.values.push(convert(self, i)?);
        }

        Ok(row)
    }
}

impl<'a> ToColumnNames for my::Stmt<'a> {
    fn to_column_names<'b>(&'b self) -> ColumnNames {
        let mut names = ColumnNames::default();

        if let Some(columns) = self.columns_ref() {
            for column in columns {
                names.names.push(String::from(column.name_str()));
            }
        }

        names
    }
}

impl From<my::error::Error> for Error {
    fn from(e: my::error::Error) -> Error {
        Error::QueryError(e.into())
    }
}

impl From<std::string::FromUtf8Error> for Error {
    fn from(e: std::string::FromUtf8Error) -> Error {
        Error::QueryError(e.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mysql::OptsBuilder;
    use std::env;

    fn get_config() -> OptsBuilder {
        let mut config = OptsBuilder::new();
        config.ip_or_hostname(env::var("TEST_MYSQL_HOST").ok());
        config.tcp_port(env::var("TEST_MYSQL_PORT").unwrap().parse::<u16>().unwrap());
        config.db_name(env::var("TEST_MYSQL_DB").ok());
        config.pass(env::var("TEST_MYSQL_PASSWORD").ok());
        config.user(env::var("TEST_MYSQL_USER").ok());
        config
    }

    #[test]
    fn should_provide_a_database_connection() {
        let connector = Mysql::new(get_config()).unwrap();

        connector
            .with_connection("TEST", |connection| {
                let res = connection.query_raw(
                    "select * from information_schema.`COLUMNS` where COLUMN_NAME = 'unknown_123'",
                    &[],
                )?;

                // No results expected.
                assert_eq!(res.into_iter().next().is_none(), true);

                Ok(())
            })
            .unwrap()
    }

    #[test]
    fn should_provide_a_database_transaction() {
        let connector = Mysql::new(get_config()).unwrap();

        connector
            .with_transaction("TEST", |transaction| {
                let res = transaction.query_raw(
                    "select * from information_schema.`COLUMNS` where COLUMN_NAME = 'unknown_123'",
                    &[],
                )?;

                // No results expected.
                assert_eq!(res.into_iter().next().is_none(), true);

                Ok(())
            })
            .unwrap()
    }

    const TABLE_DEF: &str = r#"
CREATE TABLE `user`(
    id       int4    PRIMARY KEY     NOT NULL,
    name     text    NOT NULL,
    age      int4    NOT NULL,
    salary   float4
);
"#;

    const CREATE_USER: &str = r#"
INSERT INTO `user` (id, name, age, salary)
VALUES (1, 'Joe', 27, 20000.00 );
"#;

    const DROP_TABLE: &str = "DROP TABLE IF EXISTS `user`;";

    #[test]
    fn should_map_columns_correctly() {
        let connector = Mysql::new(get_config()).unwrap();

        connector
            .with_connection("TEST", |connection| {
                connection.query_raw(DROP_TABLE, &[]).unwrap();
                connection.query_raw(TABLE_DEF, &[]).unwrap();
                connection.query_raw(CREATE_USER, &[]).unwrap();

                let res = connection.query_raw("SELECT * FROM `user`", &[]).unwrap();

                let mut result_count: u32 = 0;

                // Exactly one result expected.
                for row in &res {
                    assert_eq!(row.get_as_integer("id")?, 1);
                    assert_eq!(row.get_as_string("name")?, "Joe");
                    assert_eq!(row.get_as_integer("age")?, 27);
                    assert_eq!(row.get_as_real("salary")?, 20000.0);
                    result_count = result_count + 1;
                }

                assert_eq!(result_count, 1);

                Ok(())
            })
            .unwrap()
    }

}