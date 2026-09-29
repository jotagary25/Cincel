//! A minimal SQLite database writer for the `state.vscdb` of VS Code-based
//! profiles: one `ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value
//! BLOB)` with a handful of rows, exactly the schema those editors create.
//!
//! Antigravity IDE shows a sign-in onboarding on a fresh profile until the
//! application-scope key `antigravityOnboarding` is set; writing it here lets
//! the bench open the editor without an account and without network. Only
//! small tables are supported (every row and the index fit in one page).

use anyhow::{Result, bail};

const PAGE: usize = 4096;
const SCHEMA_SQL: &str = "CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB)";

/// A value of a record.
enum Value<'a> {
    Null,
    Int(u8),
    Text(&'a str),
}

fn varint(mut value: u64, out: &mut Vec<u8>) {
    // SQLite varints: big-endian groups of 7 bits, high bit = "more follows"
    // (the 9-byte form is never needed for the small values written here).
    let mut groups = Vec::new();
    loop {
        groups.push((value & 0x7F) as u8);
        value >>= 7;
        if value == 0 {
            break;
        }
    }
    for (index, group) in groups.iter().rev().enumerate() {
        let more = index + 1 < groups.len();
        out.push(if more { group | 0x80 } else { *group });
    }
}

fn record(values: &[Value<'_>]) -> Vec<u8> {
    let mut types = Vec::new();
    let mut body = Vec::new();
    for value in values {
        match value {
            Value::Null => varint(0, &mut types),
            Value::Int(n) => {
                varint(1, &mut types);
                body.push(*n);
            }
            Value::Text(text) => {
                varint(13 + 2 * text.len() as u64, &mut types);
                body.extend_from_slice(text.as_bytes());
            }
        }
    }
    // The header size counts itself (one byte while the header is small).
    let mut out = Vec::new();
    varint(types.len() as u64 + 1, &mut out);
    out.extend(types);
    out.extend(body);
    out
}

/// Writes a b-tree leaf page (`kind` 0x0D table, 0x0A index) whose header
/// starts at `start` (100 on page 1).
fn leaf_page(page: &mut [u8], start: usize, kind: u8, cells: &[Vec<u8>]) -> Result<()> {
    let mut content = PAGE;
    let mut pointers = Vec::with_capacity(cells.len());
    for cell in cells {
        content -= cell.len();
        page[content..content + cell.len()].copy_from_slice(cell);
        pointers.push(content as u16);
    }
    if start + 8 + 2 * cells.len() > content {
        bail!("state.vscdb: demasiados datos para una página");
    }
    page[start] = kind;
    page[start + 3..start + 5].copy_from_slice(&(cells.len() as u16).to_be_bytes());
    page[start + 5..start + 7].copy_from_slice(&(content as u16).to_be_bytes());
    for (index, pointer) in pointers.iter().enumerate() {
        let at = start + 8 + 2 * index;
        page[at..at + 2].copy_from_slice(&pointer.to_be_bytes());
    }
    Ok(())
}

fn table_cell(rowid: u64, payload: Vec<u8>) -> Vec<u8> {
    let mut cell = Vec::new();
    varint(payload.len() as u64, &mut cell);
    varint(rowid, &mut cell);
    cell.extend(payload);
    cell
}

fn index_cell(payload: Vec<u8>) -> Vec<u8> {
    let mut cell = Vec::new();
    varint(payload.len() as u64, &mut cell);
    cell.extend(payload);
    cell
}

/// The bytes of a `state.vscdb` with `entries` (key, text value).
pub fn item_table(entries: &[(&str, &str)]) -> Result<Vec<u8>> {
    let mut entries: Vec<(&str, &str)> = entries.to_vec();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    entries.dedup_by(|a, b| a.0 == b.0);
    if entries.len() > 100
        || entries
            .iter()
            .any(|(key, value)| key.len() + value.len() > 1000)
    {
        bail!("state.vscdb: valor demasiado largo");
    }
    let mut db = vec![0u8; PAGE * 3];

    // Page 1: header + sqlite_schema (table on page 2, its index on page 3).
    let header = &mut db[..100];
    header[..16].copy_from_slice(b"SQLite format 3\0");
    header[16..18].copy_from_slice(&(PAGE as u16).to_be_bytes());
    header[18] = 1; // write version: legacy
    header[19] = 1; // read version: legacy
    header[20] = 0; // reserved bytes per page
    header[21] = 64; // max embedded payload fraction
    header[22] = 32; // min embedded payload fraction
    header[23] = 32; // leaf payload fraction
    header[24..28].copy_from_slice(&1u32.to_be_bytes()); // change counter
    header[28..32].copy_from_slice(&3u32.to_be_bytes()); // pages
    header[40..44].copy_from_slice(&1u32.to_be_bytes()); // schema cookie
    header[44..48].copy_from_slice(&4u32.to_be_bytes()); // schema format
    header[56..60].copy_from_slice(&1u32.to_be_bytes()); // UTF-8
    header[92..96].copy_from_slice(&1u32.to_be_bytes()); // version-valid-for
    header[96..100].copy_from_slice(&3_045_001u32.to_be_bytes()); // SQLite 3.45.1

    let schema = vec![
        table_cell(
            1,
            record(&[
                Value::Text("table"),
                Value::Text("ItemTable"),
                Value::Text("ItemTable"),
                Value::Int(2),
                Value::Text(SCHEMA_SQL),
            ]),
        ),
        table_cell(
            2,
            record(&[
                Value::Text("index"),
                Value::Text("sqlite_autoindex_ItemTable_1"),
                Value::Text("ItemTable"),
                Value::Int(3),
                Value::Null,
            ]),
        ),
    ];
    leaf_page(&mut db[..PAGE], 100, 0x0D, &schema)?;

    let rows: Vec<Vec<u8>> = entries
        .iter()
        .enumerate()
        .map(|(index, (key, value))| {
            table_cell(
                index as u64 + 1,
                record(&[Value::Text(key), Value::Text(value)]),
            )
        })
        .collect();
    leaf_page(&mut db[PAGE..2 * PAGE], 0, 0x0D, &rows)?;

    // The UNIQUE index: (key, rowid), sorted by key.
    let index: Vec<Vec<u8>> = entries
        .iter()
        .enumerate()
        .map(|(position, (key, _))| {
            index_cell(record(&[Value::Text(key), Value::Int(position as u8 + 1)]))
        })
        .collect();
    leaf_page(&mut db[2 * PAGE..], 0, 0x0A, &index)?;
    Ok(db)
}
