//! The mysqli extension over crate::mysql: the procedural functions, the
//! mysqli and mysqli_result classes, error reporting modes and
//! mysqli_sql_exception

use std::cell::RefCell;
use std::rc::Rc;

use super::{arg_int, arg_str, args, Runtime};
use crate::array::{Key, PhpArray};
use crate::diag;
use crate::interp::{EResult, Fatal};
use crate::mysql::{self, Column, Connection};
use crate::object::{Internal, Object};
use crate::value::Value;

pub const NAMES: &[&str] = &[
    "mysqli_init", "mysqli_connect", "mysqli_real_connect", "mysqli_connect_errno", "mysqli_connect_error",
    "mysqli_select_db", "mysqli_set_charset", "mysqli_character_set_name", "mysqli_query", "mysqli_real_query",
    "mysqli_multi_query", "mysqli_more_results", "mysqli_next_result", "mysqli_store_result", "mysqli_use_result",
    "mysqli_fetch_assoc", "mysqli_fetch_row", "mysqli_fetch_array", "mysqli_fetch_object", "mysqli_fetch_all",
    "mysqli_fetch_column", "mysqli_fetch_field", "mysqli_fetch_fields", "mysqli_num_rows", "mysqli_num_fields",
    "mysqli_field_count", "mysqli_free_result", "mysqli_data_seek", "mysqli_field_seek", "mysqli_affected_rows",
    "mysqli_insert_id", "mysqli_errno", "mysqli_error", "mysqli_sqlstate", "mysqli_real_escape_string",
    "mysqli_escape_string", "mysqli_get_server_info", "mysqli_get_server_version", "mysqli_get_client_info",
    "mysqli_get_client_version", "mysqli_get_host_info", "mysqli_get_proto_info", "mysqli_thread_id", "mysqli_ping",
    "mysqli_close", "mysqli_report", "mysqli_options", "mysqli_set_opt", "mysqli_autocommit",
    "mysqli_begin_transaction", "mysqli_commit", "mysqli_rollback", "mysqli_info", "mysqli_warning_count",
];

const MYSQLI_ASSOC: i64 = 1;
const MYSQLI_NUM: i64 = 2;
const MYSQLI_BOTH: i64 = 3;
const REPORT_ERROR: i64 = 1;
const REPORT_STRICT: i64 = 2;
const CLIENT_VERSION: &str = "mysqlnd 8.5.3";
const CLIENT_VERSION_ID: i64 = 80503;

pub fn constant(name: &str) -> Option<Value> {
    Some(Value::Int(match name {
        "MYSQLI_ASSOC" => MYSQLI_ASSOC,
        "MYSQLI_NUM" => MYSQLI_NUM,
        "MYSQLI_BOTH" => MYSQLI_BOTH,
        "MYSQLI_REPORT_OFF" => 0,
        "MYSQLI_REPORT_ERROR" => REPORT_ERROR,
        "MYSQLI_REPORT_STRICT" => REPORT_STRICT,
        "MYSQLI_REPORT_INDEX" => 4,
        "MYSQLI_REPORT_ALL" => 255,
        "MYSQLI_STORE_RESULT" => 0,
        "MYSQLI_USE_RESULT" => 1,
        "MYSQLI_CLIENT_FOUND_ROWS" => 2,
        "MYSQLI_CLIENT_COMPRESS" => 32,
        "MYSQLI_CLIENT_SSL" => 2048,
        "MYSQLI_CLIENT_INTERACTIVE" => 1024,
        "MYSQLI_CLIENT_IGNORE_SPACE" => 256,
        "MYSQLI_CLIENT_NO_SCHEMA" => 16,
        "MYSQLI_CLIENT_SSL_DONT_VERIFY_SERVER_CERT" => 64,
        "MYSQLI_OPT_CONNECT_TIMEOUT" => 0,
        "MYSQLI_OPT_LOCAL_INFILE" => 8,
        "MYSQLI_INIT_COMMAND" => 3,
        "MYSQLI_OPT_READ_TIMEOUT" => 11,
        "MYSQLI_OPT_INT_AND_FLOAT_NATIVE" => 201,
        "MYSQLI_TRANS_START_READ_ONLY" => 4,
        "MYSQLI_TRANS_START_READ_WRITE" => 2,
        "MYSQLI_TRANS_START_WITH_CONSISTENT_SNAPSHOT" => 1,
        "MYSQLI_NOT_NULL_FLAG" => 1,
        "MYSQLI_PRI_KEY_FLAG" => 2,
        "MYSQLI_UNIQUE_KEY_FLAG" => 4,
        "MYSQLI_MULTIPLE_KEY_FLAG" => 8,
        "MYSQLI_BLOB_FLAG" => 16,
        "MYSQLI_UNSIGNED_FLAG" => 32,
        "MYSQLI_ZEROFILL_FLAG" => 64,
        "MYSQLI_AUTO_INCREMENT_FLAG" => 512,
        "MYSQLI_TIMESTAMP_FLAG" => 1024,
        "MYSQLI_SET_FLAG" => 2048,
        "MYSQLI_NUM_FLAG" => 32768,
        "MYSQLI_BINARY_FLAG" => 128,
        "MYSQLI_ENUM_FLAG" => 256,
        "MYSQLI_TYPE_DECIMAL" => 0,
        "MYSQLI_TYPE_TINY" => 1,
        "MYSQLI_TYPE_SHORT" => 2,
        "MYSQLI_TYPE_LONG" => 3,
        "MYSQLI_TYPE_FLOAT" => 4,
        "MYSQLI_TYPE_DOUBLE" => 5,
        "MYSQLI_TYPE_NULL" => 6,
        "MYSQLI_TYPE_TIMESTAMP" => 7,
        "MYSQLI_TYPE_LONGLONG" => 8,
        "MYSQLI_TYPE_INT24" => 9,
        "MYSQLI_TYPE_DATE" => 10,
        "MYSQLI_TYPE_TIME" => 11,
        "MYSQLI_TYPE_DATETIME" => 12,
        "MYSQLI_TYPE_YEAR" => 13,
        "MYSQLI_TYPE_NEWDATE" => 14,
        "MYSQLI_TYPE_BIT" => 16,
        "MYSQLI_TYPE_JSON" => 245,
        "MYSQLI_TYPE_NEWDECIMAL" => 246,
        "MYSQLI_TYPE_ENUM" => 247,
        "MYSQLI_TYPE_SET" => 248,
        "MYSQLI_TYPE_TINY_BLOB" => 249,
        "MYSQLI_TYPE_MEDIUM_BLOB" => 250,
        "MYSQLI_TYPE_LONG_BLOB" => 251,
        "MYSQLI_TYPE_BLOB" => 252,
        "MYSQLI_TYPE_VAR_STRING" => 253,
        "MYSQLI_TYPE_STRING" => 254,
        "MYSQLI_TYPE_CHAR" => 1,
        "MYSQLI_TYPE_INTERVAL" => 247,
        "MYSQLI_TYPE_GEOMETRY" => 255,
        _ => return None,
    }))
}

/// A connection's state between calls
#[derive(Default)]
pub struct Link {
    conn: Option<Connection>,
    host: String,
    errno: u16,
    error: String,
    sqlstate: String,
    affected_rows: i64,
    insert_id: u64,
    field_count: usize,
    warnings: u16,
    info: Vec<u8>,
    /// What the last query produced, until store_result() takes it
    last: Option<mysql::QueryResult>,
    closed: bool,
}

pub struct ResultSet {
    columns: Vec<Column>,
    rows: Vec<Vec<Option<Vec<u8>>>>,
    /// The next row to fetch and the next field for fetch_field()
    row: usize,
    field: usize,
    freed: bool,
}

pub enum State {
    None,
    Link(Link),
    Result(ResultSet),
}

thread_local! {
    /// mysqli_report()'s mode: errors throw by default since PHP 8.1
    static REPORT: RefCell<i64> = const { RefCell::new(REPORT_ERROR | REPORT_STRICT) };
    /// The last connection error, for mysqli_connect_errno() and _error()
    static CONNECT_ERROR: RefCell<(u16, String)> = const { RefCell::new((0, String::new())) };
}

fn report_mode() -> i64 {
    REPORT.with(|r| *r.borrow())
}

fn state(o: &Object) -> Option<&RefCell<State>> {
    match &o.internal {
        Internal::Mysqli(s) => Some(s),
        _ => None,
    }
}

/// Throws mysqli_sql_exception in strict mode, or warns
fn sql_error(fname: &str, e: &mysql::Error) -> EResult<()> {
    if report_mode() & REPORT_STRICT != 0 {
        let mut f = Fatal::new("mysqli_sql_exception", e.message.clone());
        f.code = e.code as i64;
        return Err(f);
    }
    diag::warning(format!("{fname}(): ({}/{}): {}", e.sqlstate, e.code, e.message));
    Ok(())
}

fn link_arg(fname: &str, v: &Value) -> EResult<Rc<Object>> {
    match v.copy_out() {
        Value::Object(o) if o.class.is_subclass_of("mysqli") => Ok(o),
        other => Err(Fatal::type_error(format!(
            "{fname}(): Argument #1 ($mysql) must be of type mysqli, {} given",
            other.value_name()
        ))),
    }
}

fn result_arg(fname: &str, v: &Value) -> EResult<Rc<Object>> {
    match v.copy_out() {
        Value::Object(o) if o.class.is_subclass_of("mysqli_result") => Ok(o),
        other => Err(Fatal::type_error(format!(
            "{fname}(): Argument #1 ($result) must be of type mysqli_result, {} given",
            other.value_name()
        ))),
    }
}

/// Runs `f` on a connected link, or fails as php-src does for a closed one
fn with_link<T>(o: &Object, f: impl FnOnce(&mut Link) -> T) -> EResult<T> {
    let st = state(o).ok_or_else(|| Fatal::error("mysqli object is not fully initialized"))?;
    let mut st = st.borrow_mut();
    match &mut *st {
        State::Link(l) if l.closed => Err(Fatal::error("mysqli object is already closed")),
        State::Link(l) => Ok(f(l)),
        _ => Err(Fatal::error("mysqli object is not fully initialized")),
    }
}

fn with_result<T>(o: &Object, f: impl FnOnce(&mut ResultSet) -> T) -> EResult<T> {
    let st = state(o).ok_or_else(|| Fatal::error("mysqli_result object is already closed"))?;
    let mut st = st.borrow_mut();
    match &mut *st {
        State::Result(r) if !r.freed => Ok(f(r)),
        _ => Err(Fatal::error("mysqli_result object is already closed")),
    }
}

fn opt_str(rt: &mut dyn Runtime, fname: &str, n: usize, param: &str, v: Option<Value>) -> EResult<String> {
    match v {
        None | Some(Value::Null) => Ok(String::new()),
        Some(v) => Ok(String::from_utf8_lossy(&arg_str(rt, fname, n, param, &v)?).into_owned()),
    }
}

/// mysqli_real_connect on an initialized link
fn real_connect(rt: &mut dyn Runtime, fname: &str, link: &Rc<Object>, vals: Vec<Value>) -> EResult<bool> {
    let mut it = vals.into_iter();
    let host = opt_str(rt, fname, 2, "hostname", it.next())?;
    let user = opt_str(rt, fname, 3, "username", it.next())?;
    let pass = match it.next() {
        None | Some(Value::Null) => Vec::new(),
        Some(v) => arg_str(rt, fname, 4, "password", &v)?,
    };
    let db = opt_str(rt, fname, 5, "database", it.next())?;
    let port = match it.next() {
        None | Some(Value::Null) => 3306,
        Some(v) => arg_int(fname, 6, "port", &v)?,
    };
    let socket = opt_str(rt, fname, 7, "socket", it.next())?;
    // `host:port` and `host:/socket` are accepted, as by wpdb
    let (host, port, socket) = match host.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') && p.parse::<u16>().is_ok() => (h.to_string(), p.parse::<u16>().unwrap_or(3306), socket),
        _ => (host, port as u16, socket),
    };
    let user = if user.is_empty() { std::env::var("USER").unwrap_or_default() } else { user };
    match Connection::connect(&host, port, &socket, &user, &pass, &db) {
        Ok(conn) => {
            CONNECT_ERROR.with(|e| *e.borrow_mut() = (0, String::new()));
            let host_info = if host.is_empty() || host == "localhost" { "Localhost via UNIX socket".to_string() } else { format!("{host} via TCP/IP") };
            if let Some(st) = state(link) {
                *st.borrow_mut() = State::Link(Link { conn: Some(conn), host: host_info, sqlstate: "00000".into(), ..Default::default() });
            }
            Ok(true)
        }
        Err(e) => {
            CONNECT_ERROR.with(|c| *c.borrow_mut() = (e.code, e.message.clone()));
            if let Some(st) = state(link) {
                *st.borrow_mut() = State::Link(Link { errno: e.code, error: e.message.clone(), sqlstate: e.sqlstate.clone(), ..Default::default() });
            }
            if report_mode() & REPORT_STRICT != 0 {
                let mut f = Fatal::new("mysqli_sql_exception", e.message.clone());
                f.code = e.code as i64;
                return Err(f);
            }
            diag::warning(format!("{fname}(): ({}/{}): {}", e.sqlstate, e.code, e.message));
            Ok(false)
        }
    }
}

/// Records a statement's outcome on the link
fn record(l: &mut Link, r: &Result<mysql::QueryResult, mysql::Error>) {
    match r {
        Ok(q) => {
            l.errno = 0;
            l.error.clear();
            l.sqlstate = "00000".into();
            l.warnings = q.warnings;
            l.info = q.info.clone();
            match &q.columns {
                Some(cols) => {
                    l.field_count = cols.len();
                    l.affected_rows = q.rows.len() as i64;
                }
                None => {
                    l.field_count = 0;
                    l.affected_rows = q.affected_rows as i64;
                    l.insert_id = q.insert_id;
                }
            }
        }
        Err(e) => {
            l.errno = e.code;
            l.error = e.message.clone();
            l.sqlstate = e.sqlstate.clone();
            l.affected_rows = -1;
        }
    }
}

/// A mysqli_result object for rows
fn new_result(rt: &mut dyn Runtime, q: mysql::QueryResult) -> Value {
    let Some(obj) = rt.new_object("mysqli_result") else { return Value::Bool(false) };
    if let Some(st) = state(&obj) {
        *st.borrow_mut() =
            State::Result(ResultSet { columns: q.columns.unwrap_or_default(), rows: q.rows, row: 0, field: 0, freed: false });
    }
    Value::Object(obj)
}

fn cell(v: &Option<Vec<u8>>) -> Value {
    v.as_ref().map_or(Value::Null, |s| Value::str(s.clone()))
}

/// The next row as an array of the given kind; None after the last
fn fetch(r: &mut ResultSet, mode: i64) -> Option<PhpArray> {
    let row = r.rows.get(r.row)?;
    let mut out = PhpArray::new();
    for (i, (col, v)) in r.columns.iter().zip(row).enumerate() {
        if mode & MYSQLI_NUM != 0 {
            out.insert(Key::Int(i as i64), cell(v));
        }
        if mode & MYSQLI_ASSOC != 0 {
            out.insert(Key::from_bytes(&col.name), cell(v));
        }
    }
    r.row += 1;
    Some(out)
}

fn field_object(rt: &mut dyn Runtime, r: &ResultSet, i: usize) -> Value {
    let Some(c) = r.columns.get(i) else { return Value::Bool(false) };
    let Some(o) = rt.new_object("stdClass") else { return Value::Bool(false) };
    let max_length = r.rows.iter().map(|row| row.get(i).and_then(|v| v.as_ref()).map_or(0, Vec::len)).max().unwrap_or(0);
    o.set("name", Value::str(c.name.clone()));
    o.set("orgname", Value::str(c.org_name.clone()));
    o.set("table", Value::str(c.table.clone()));
    o.set("orgtable", Value::str(c.org_table.clone()));
    o.set("def", Value::str(""));
    o.set("db", Value::str(c.db.clone()));
    o.set("catalog", Value::str(c.catalog.clone()));
    o.set("max_length", Value::Int(max_length as i64));
    o.set("length", Value::Int(c.length as i64));
    o.set("charsetnr", Value::Int(c.charset as i64));
    o.set("flags", Value::Int(c.flags as i64));
    o.set("type", Value::Int(c.ty as i64));
    o.set("decimals", Value::Int(c.decimals as i64));
    Value::Object(o)
}

/// A property of a mysqli or mysqli_result object; None for other names
pub fn property(o: &Object, name: &str) -> Option<Value> {
    let st = state(o)?;
    let st = st.borrow();
    match &*st {
        State::Link(l) => Some(match name {
            "affected_rows" => Value::Int(l.affected_rows),
            "insert_id" => Value::Int(l.insert_id as i64),
            "errno" => Value::Int(l.errno as i64),
            "error" => Value::str(l.error.clone()),
            "sqlstate" => Value::str(l.sqlstate.clone()),
            "field_count" => Value::Int(l.field_count as i64),
            "warning_count" => Value::Int(l.warnings as i64),
            "info" => if l.info.is_empty() { Value::Null } else { Value::str(l.info.clone()) },
            "server_info" => Value::str(l.conn.as_ref().map(|c| c.server_version.clone()).unwrap_or_default()),
            "server_version" => Value::Int(l.conn.as_ref().map_or(0, Connection::server_version_number)),
            "client_info" => Value::str(CLIENT_VERSION),
            "client_version" => Value::Int(CLIENT_VERSION_ID),
            "host_info" => Value::str(l.host.clone()),
            "protocol_version" => Value::Int(10),
            "thread_id" => Value::Int(l.conn.as_ref().map_or(0, |c| c.thread_id as i64)),
            "connect_errno" => Value::Int(CONNECT_ERROR.with(|e| e.borrow().0 as i64)),
            "connect_error" => CONNECT_ERROR.with(|e| {
                let e = e.borrow();
                if e.0 == 0 { Value::Null } else { Value::str(e.1.clone()) }
            }),
            _ => return None,
        }),
        State::Result(r) => Some(match name {
            "num_rows" => Value::Int(r.rows.len() as i64),
            "field_count" => Value::Int(r.columns.len() as i64),
            "current_field" => Value::Int(r.field as i64),
            "lengths" => match r.row.checked_sub(1).and_then(|i| r.rows.get(i)) {
                Some(row) => super::list(row.iter().map(|v| Value::Int(v.as_ref().map_or(0, Vec::len) as i64))),
                None => Value::Null,
            },
            "type" => Value::Int(0),
            _ => return None,
        }),
        State::None => None,
    }
}

pub fn call(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> Option<EResult<Value>> {
    Some(call_inner(rt, name, lname, vals))
}

/// A method of mysqli or mysqli_result: the procedural function with the object first
pub fn method(rt: &mut dyn Runtime, this: &Rc<Object>, method: &str, vals: Vec<Value>) -> EResult<Value> {
    let is_result = this.class.is_subclass_of("mysqli_result");
    let fname = match (is_result, method) {
        (false, "__construct") => {
            if vals.is_empty() {
                *state(this).expect("mysqli state").borrow_mut() = State::Link(Link::default());
                return Ok(Value::Null);
            }
            real_connect(rt, "mysqli::__construct", this, vals)?;
            return Ok(Value::Null);
        }
        (false, "connect") => "mysqli_real_connect",
        (true, "free" | "close" | "free_result") => "mysqli_free_result",
        (false, m) => return call_named(rt, &format!("mysqli_{m}"), this, vals),
        (true, m) => return call_named(rt, &format!("mysqli_{m}"), this, vals),
    };
    call_named(rt, fname, this, vals)
}

fn call_named(rt: &mut dyn Runtime, fname: &str, this: &Rc<Object>, vals: Vec<Value>) -> EResult<Value> {
    if !NAMES.contains(&fname) {
        return Err(Fatal::error(format!("Call to undefined method {}::{}()", this.class.name, &fname[7..])));
    }
    let mut all = vec![Value::Object(this.clone())];
    all.extend(vals);
    call_inner(rt, fname, fname, all)
}

fn call_inner(rt: &mut dyn Runtime, name: &str, lname: &str, vals: Vec<Value>) -> EResult<Value> {
    match lname {
        "mysqli_init" => {
            args::<0, 0>(name, vals)?;
            let Some(o) = rt.new_object("mysqli") else { return Ok(Value::Bool(false)) };
            *state(&o).expect("mysqli state").borrow_mut() = State::Link(Link::default());
            Ok(Value::Object(o))
        }
        "mysqli_connect" => {
            let Some(o) = rt.new_object("mysqli") else { return Ok(Value::Bool(false)) };
            let mut all = vec![Value::Object(o.clone())];
            all.extend(vals);
            Ok(if real_connect(rt, lname, &o, all.split_off(1))? { Value::Object(o) } else { Value::Bool(false) })
        }
        "mysqli_real_connect" => {
            let mut it = vals.into_iter();
            let Some(link) = it.next() else { return Err(super::arg_count(name, 1, 0)) };
            let link = link_arg(lname, &link)?;
            Ok(Value::Bool(real_connect(rt, lname, &link, it.take(6).collect())?))
        }
        "mysqli_connect_errno" => Ok(Value::Int(CONNECT_ERROR.with(|e| e.borrow().0 as i64))),
        "mysqli_connect_error" => Ok(CONNECT_ERROR.with(|e| {
            let e = e.borrow();
            if e.0 == 0 { Value::Null } else { Value::str(e.1.clone()) }
        })),
        "mysqli_report" => {
            let [flags] = args::<1, 1>(name, vals)?;
            let flags = arg_int(lname, 1, "flags", &flags.unwrap())?;
            REPORT.with(|r| *r.borrow_mut() = flags);
            Ok(Value::Bool(true))
        }
        "mysqli_get_client_info" => Ok(Value::str(CLIENT_VERSION)),
        "mysqli_get_client_version" => Ok(Value::Int(CLIENT_VERSION_ID)),
        "mysqli_real_escape_string" | "mysqli_escape_string" => {
            let [link, s] = args::<2, 2>(name, vals)?;
            let link = link_arg(lname, &link.unwrap())?;
            let s = arg_str(rt, lname, 2, "string", &s.unwrap())?;
            let no_bs = with_link(&link, |l| l.conn.as_ref().is_some_and(Connection::no_backslash_escapes))?;
            Ok(Value::str(mysql::escape(&s, no_bs)))
        }
        "mysqli_query" | "mysqli_real_query" | "mysqli_multi_query" => {
            let [link, sql, mode] = args::<2, 3>(name, vals)?;
            let link = link_arg(lname, &link.unwrap())?;
            let sql = arg_str(rt, lname, 2, "query", &sql.unwrap())?;
            let _ = mode;
            if sql.is_empty() {
                return Err(Fatal::value_error(format!("{lname}(): Argument #2 ($query) must not be empty")));
            }
            let r = with_link(&link, |l| {
                let r = match &mut l.conn {
                    Some(c) => c.query(&sql),
                    None => Err(mysql::Error { code: 2006, sqlstate: "HY000".into(), message: "MySQL server has gone away".into() }),
                };
                record(l, &r);
                r
            })?;
            match r {
                Err(e) => {
                    if report_mode() & REPORT_ERROR != 0 {
                        sql_error(lname, &e)?;
                    }
                    Ok(Value::Bool(false))
                }
                Ok(q) if lname == "mysqli_query" && q.columns.is_some() => Ok(new_result(rt, q)),
                Ok(q) => {
                    if q.columns.is_some() {
                        with_link(&link, |l| l.last = Some(q))?;
                    }
                    Ok(Value::Bool(true))
                }
            }
        }
        "mysqli_store_result" | "mysqli_use_result" => {
            let [link, _] = args::<1, 2>(name, vals)?;
            let link = link_arg(lname, &link.unwrap())?;
            match with_link(&link, |l| l.last.take())? {
                Some(q) => Ok(new_result(rt, q)),
                None => Ok(Value::Bool(false)),
            }
        }
        "mysqli_more_results" => {
            let [link] = args::<1, 1>(name, vals)?;
            let link = link_arg(lname, &link.unwrap())?;
            Ok(Value::Bool(with_link(&link, |l| l.conn.as_ref().is_some_and(Connection::more_results))?))
        }
        "mysqli_next_result" => {
            let [link] = args::<1, 1>(name, vals)?;
            let link = link_arg(lname, &link.unwrap())?;
            let r = with_link(&link, |l| {
                let more = l.conn.as_ref().is_some_and(Connection::more_results);
                if !more {
                    return None;
                }
                let r = l.conn.as_mut().map(Connection::next_result)?;
                record(l, &r);
                Some(r)
            })?;
            match r {
                None => Ok(Value::Bool(false)),
                Some(Ok(q)) => {
                    if q.columns.is_some() {
                        with_link(&link, |l| l.last = Some(q))?;
                    }
                    Ok(Value::Bool(true))
                }
                Some(Err(e)) => {
                    if report_mode() & REPORT_ERROR != 0 {
                        sql_error(lname, &e)?;
                    }
                    Ok(Value::Bool(false))
                }
            }
        }
        "mysqli_select_db" => {
            let [link, db] = args::<2, 2>(name, vals)?;
            let link = link_arg(lname, &link.unwrap())?;
            let db = String::from_utf8_lossy(&arg_str(rt, lname, 2, "database", &db.unwrap())?).into_owned();
            let r = with_link(&link, |l| {
                let r = l.conn.as_mut().map_or(Ok(()), |c| c.select_db(&db));
                if let Err(e) = &r {
                    l.errno = e.code;
                    l.error = e.message.clone();
                    l.sqlstate = e.sqlstate.clone();
                } else {
                    l.errno = 0;
                    l.error.clear();
                }
                r
            })?;
            match r {
                Ok(()) => Ok(Value::Bool(true)),
                Err(e) => {
                    if report_mode() & REPORT_ERROR != 0 {
                        sql_error(lname, &e)?;
                    }
                    Ok(Value::Bool(false))
                }
            }
        }
        "mysqli_set_charset" => {
            let [link, cs] = args::<2, 2>(name, vals)?;
            let link = link_arg(lname, &link.unwrap())?;
            let cs = arg_str(rt, lname, 2, "charset", &cs.unwrap())?;
            let mut sql = b"SET NAMES ".to_vec();
            sql.extend_from_slice(&cs);
            let r = with_link(&link, |l| l.conn.as_mut().map(|c| c.query(&sql)))?;
            Ok(Value::Bool(matches!(r, Some(Ok(_)))))
        }
        "mysqli_character_set_name" => {
            let [link] = args::<1, 1>(name, vals)?;
            let link = link_arg(lname, &link.unwrap())?;
            let r = with_link(&link, |l| l.conn.as_mut().map(|c| c.query(b"SELECT @@character_set_client")))?;
            Ok(match r {
                Some(Ok(q)) => q.rows.first().and_then(|r| r.first()).map_or(Value::str("utf8mb4"), cell),
                _ => Value::str("utf8mb4"),
            })
        }
        "mysqli_get_server_info" | "mysqli_get_server_version" | "mysqli_get_host_info" | "mysqli_get_proto_info"
        | "mysqli_thread_id" | "mysqli_affected_rows" | "mysqli_insert_id" | "mysqli_errno" | "mysqli_error"
        | "mysqli_sqlstate" | "mysqli_field_count" | "mysqli_info" | "mysqli_warning_count" => {
            let [link] = args::<1, 1>(name, vals)?;
            let link = link_arg(lname, &link.unwrap())?;
            let prop = match lname {
                "mysqli_get_server_info" => "server_info",
                "mysqli_get_server_version" => "server_version",
                "mysqli_get_host_info" => "host_info",
                "mysqli_get_proto_info" => "protocol_version",
                other => &other[7..],
            };
            with_link(&link, |_| ())?;
            Ok(property(&link, prop).unwrap_or(Value::Null))
        }
        "mysqli_ping" => {
            let [link] = args::<1, 1>(name, vals)?;
            let link = link_arg(lname, &link.unwrap())?;
            Ok(Value::Bool(with_link(&link, |l| l.conn.as_mut().is_some_and(|c| c.ping().is_ok()))?))
        }
        "mysqli_close" => {
            let [link] = args::<1, 1>(name, vals)?;
            let link = link_arg(lname, &link.unwrap())?;
            with_link(&link, |l| {
                if let Some(mut c) = l.conn.take() {
                    c.close();
                }
                l.closed = true;
            })?;
            Ok(Value::Bool(true))
        }
        "mysqli_options" | "mysqli_set_opt" => {
            args::<3, 3>(name, vals)?;
            Ok(Value::Bool(true))
        }
        "mysqli_autocommit" | "mysqli_begin_transaction" | "mysqli_commit" | "mysqli_rollback" => {
            let mut it = vals.into_iter();
            let Some(link) = it.next() else { return Err(super::arg_count(name, 1, 0)) };
            let link = link_arg(lname, &link)?;
            let sql: &[u8] = match lname {
                "mysqli_autocommit" => {
                    if it.next().is_some_and(|v| v.to_bool()) { b"SET autocommit=1" } else { b"SET autocommit=0" }
                }
                "mysqli_begin_transaction" => b"START TRANSACTION",
                "mysqli_commit" => b"COMMIT",
                _ => b"ROLLBACK",
            };
            let r = with_link(&link, |l| l.conn.as_mut().map(|c| c.query(sql)))?;
            Ok(Value::Bool(matches!(r, Some(Ok(_)))))
        }
        "mysqli_fetch_assoc" | "mysqli_fetch_row" | "mysqli_fetch_array" => {
            let [res, mode] = args::<1, 2>(name, vals)?;
            let res = result_arg(lname, &res.unwrap())?;
            let mode = match (lname, mode) {
                ("mysqli_fetch_assoc", _) => MYSQLI_ASSOC,
                ("mysqli_fetch_row", _) => MYSQLI_NUM,
                (_, Some(m)) => arg_int(lname, 2, "mode", &m)?,
                (_, None) => MYSQLI_BOTH,
            };
            if !(1..=3).contains(&mode) {
                return Err(Fatal::value_error(format!(
                    "{lname}(): Argument #2 ($mode) must be one of MYSQLI_NUM, MYSQLI_ASSOC, or MYSQLI_BOTH"
                )));
            }
            Ok(with_result(&res, |r| fetch(r, mode))?.map_or(Value::Null, |a| Value::Array(Rc::new(a))))
        }
        "mysqli_fetch_column" => {
            let [res, col] = args::<1, 2>(name, vals)?;
            let res = result_arg(lname, &res.unwrap())?;
            let col = match col {
                Some(c) => arg_int(lname, 2, "column", &c)?,
                None => 0,
            };
            Ok(with_result(&res, |r| {
                let row = r.rows.get(r.row)?;
                r.row += 1;
                row.get(col as usize).map(cell)
            })?
            .unwrap_or(Value::Bool(false)))
        }
        "mysqli_fetch_object" => {
            let [res, class, _args] = args::<1, 3>(name, vals)?;
            let res = result_arg(lname, &res.unwrap())?;
            let class = opt_str(rt, lname, 2, "class", class)?;
            let class = if class.is_empty() { "stdClass".to_string() } else { class };
            let Some(row) = with_result(&res, |r| fetch(r, MYSQLI_ASSOC))? else { return Ok(Value::Null) };
            let Some(o) = rt.new_object(&class) else {
                return Err(Fatal::error(format!("Class \"{class}\" not found")));
            };
            for (k, v) in row.iter() {
                let key = match k {
                    Key::Int(i) => Key::Str(i.to_string().into_bytes().into()),
                    k => k.clone(),
                };
                o.set_key(key, v.clone());
            }
            Ok(Value::Object(o))
        }
        "mysqli_fetch_all" => {
            let [res, mode] = args::<1, 2>(name, vals)?;
            let res = result_arg(lname, &res.unwrap())?;
            let mode = match mode {
                Some(m) => arg_int(lname, 2, "mode", &m)?,
                None => MYSQLI_NUM,
            };
            let rows = with_result(&res, |r| {
                let mut out = Vec::new();
                while let Some(a) = fetch(r, mode) {
                    out.push(Value::Array(Rc::new(a)));
                }
                out
            })?;
            Ok(super::list(rows))
        }
        "mysqli_fetch_field" => {
            let [res] = args::<1, 1>(name, vals)?;
            let res = result_arg(lname, &res.unwrap())?;
            let i = with_result(&res, |r| {
                let i = r.field;
                r.field += 1;
                i
            })?;
            let st = state(&res).expect("result state").borrow();
            let State::Result(r) = &*st else { return Ok(Value::Bool(false)) };
            Ok(field_object(rt, r, i))
        }
        "mysqli_fetch_fields" => {
            let [res] = args::<1, 1>(name, vals)?;
            let res = result_arg(lname, &res.unwrap())?;
            with_result(&res, |_| ())?;
            let st = state(&res).expect("result state").borrow();
            let State::Result(r) = &*st else { return Ok(Value::Bool(false)) };
            let fields: Vec<Value> = (0..r.columns.len()).map(|i| field_object(rt, r, i)).collect();
            Ok(super::list(fields))
        }
        "mysqli_num_rows" | "mysqli_num_fields" => {
            let [res] = args::<1, 1>(name, vals)?;
            let res = result_arg(lname, &res.unwrap())?;
            let n = with_result(&res, |r| if lname == "mysqli_num_rows" { r.rows.len() } else { r.columns.len() })?;
            Ok(Value::Int(n as i64))
        }
        "mysqli_data_seek" | "mysqli_field_seek" => {
            let [res, n] = args::<2, 2>(name, vals)?;
            let res = result_arg(lname, &res.unwrap())?;
            let n = arg_int(lname, 2, if lname == "mysqli_data_seek" { "offset" } else { "index" }, &n.unwrap())?;
            Ok(Value::Bool(with_result(&res, |r| {
                let len = if lname == "mysqli_data_seek" { r.rows.len() } else { r.columns.len() };
                if n < 0 || n as usize >= len {
                    return false;
                }
                if lname == "mysqli_data_seek" {
                    r.row = n as usize;
                } else {
                    r.field = n as usize;
                }
                true
            })?))
        }
        "mysqli_free_result" => {
            let [res] = args::<1, 1>(name, vals)?;
            let res = result_arg(lname, &res.unwrap())?;
            with_result(&res, |r| {
                r.freed = true;
                r.rows = Vec::new();
            })?;
            Ok(Value::Null)
        }
        _ => Err(Fatal::error(format!("Call to undefined function {name}()"))),
    }
}
