//! A dSheet's Yjs state -> sheets of display strings.
//!
//! The workbook is `Y.Array(<dsheetId>)` of sheets: `Y.Map`s in current files,
//! plain objects in older ones (fileverse-dsheets `migrate-new-yjs.ts`). Cells
//! live in `celldata`, either a `Y.Map` keyed `r_c` or a FortuneSheet array of
//! `{r, c, v}`. We show the formatted value (`m`) where the editor computed one
//! and the raw value otherwise; formulas are not re-evaluated.

use serde::Serialize;
use serde_json::Value;
use yrs::{Doc, ReadTxn, Transact};

pub const MAX_ROWS: usize = 1000;
pub const MAX_COLS: usize = 100;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sheet {
    pub name: String,
    pub rows: Vec<Vec<String>>,
    pub row_count: usize,
    pub col_count: usize,
    pub truncated: bool,
}

fn cell_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Object(o) => {
            if let Some(m) = o.get("m").filter(|m| !m.is_null()) {
                return cell_text(m);
            }
            if let Some(Value::Object(ct)) = o.get("ct") {
                if let Some(Value::Array(parts)) = ct.get("s") {
                    return parts.iter().map(|p| cell_text(p.get("v").unwrap_or(&Value::Null))).collect();
                }
            }
            o.get("v").map(cell_text).unwrap_or_default()
        }
        Value::Array(_) => String::new(),
    }
}

fn cells(celldata: &Value) -> Vec<(usize, usize, String)> {
    let mut out = vec![];
    let mut add = |e: &Value| {
        let r = e.get("r").and_then(Value::as_f64);
        let c = e.get("c").and_then(Value::as_f64);
        if let (Some(r), Some(c)) = (r, c) {
            if r >= 0.0 && c >= 0.0 {
                out.push((r as usize, c as usize, cell_text(e.get("v").unwrap_or(&Value::Null))));
            }
        }
    };
    match celldata {
        Value::Array(a) => a.iter().for_each(&mut add),
        Value::Object(o) => o.values().for_each(&mut add),
        _ => {}
    }
    out
}

fn sheet_of(v: &Value, index: usize) -> Option<(f64, Sheet)> {
    let o = v.as_object()?;
    if o.get("hide").and_then(Value::as_f64) == Some(1.0) {
        return None;
    }
    let name = o.get("name").and_then(Value::as_str).unwrap_or("Sheet").to_string();
    let order = o.get("order").and_then(Value::as_f64).unwrap_or(index as f64);
    let mut cs = cells(o.get("celldata").unwrap_or(&Value::Null));
    // Older snapshots carry a dense `data` matrix instead of sparse celldata.
    if cs.is_empty() {
        if let Some(Value::Array(rows)) = o.get("data") {
            for (r, row) in rows.iter().enumerate() {
                if let Value::Array(cols) = row {
                    for (c, cell) in cols.iter().enumerate() {
                        let t = cell_text(cell);
                        if !t.is_empty() {
                            cs.push((r, c, t));
                        }
                    }
                }
            }
        }
    }
    cs.retain(|(_, _, t)| !t.is_empty());
    let row_count = cs.iter().map(|c| c.0 + 1).max().unwrap_or(0);
    let col_count = cs.iter().map(|c| c.1 + 1).max().unwrap_or(0);
    let (nr, nc) = (row_count.min(MAX_ROWS), col_count.min(MAX_COLS));
    let mut rows = vec![vec![String::new(); nc]; nr];
    for (r, c, t) in cs {
        if r < nr && c < nc {
            rows[r][c] = t;
        }
    }
    Some((order, Sheet { name, rows, row_count, col_count, truncated: row_count > nr || col_count > nc }))
}

pub fn sheets(doc: &Doc, dsheet_id: &str) -> Result<Vec<Sheet>, String> {
    let names: Vec<String> = if dsheet_id.is_empty() {
        let txn = doc.transact();
        txn.root_refs().map(|(n, _)| n.to_string()).collect()
    } else {
        vec![dsheet_id.to_string()]
    };
    for name in names {
        let arr = doc.get_or_insert_array(name.as_str());
        let txn = doc.transact();
        let json = yrs::types::ToJson::to_json(&arr, &txn);
        let v: Value = serde_json::to_value(&json).map_err(|e| e.to_string())?;
        let Value::Array(items) = v else { continue };
        let mut out: Vec<(f64, Sheet)> = items.iter().enumerate().filter_map(|(i, s)| sheet_of(s, i)).collect();
        if out.is_empty() {
            continue;
        }
        out.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        return Ok(out.into_iter().map(|(_, s)| s).collect());
    }
    Err("no sheets found in this file".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use yrs::{Array, Map, MapPrelim, Transact};

    #[test]
    fn map_celldata() {
        let doc = Doc::new();
        let arr = doc.get_or_insert_array("wb");
        {
            let mut txn = doc.transact_mut();
            let sheet = arr.push_back(&mut txn, MapPrelim::default());
            sheet.insert(&mut txn, "name", "Budget");
            let cd = sheet.insert(&mut txn, "celldata", MapPrelim::default());
            cd.insert(&mut txn, "0_0", yrs::any!({"r": 0, "c": 0, "v": {"v": "Item", "m": "Item"}}));
            cd.insert(&mut txn, "1_1", yrs::any!({"r": 1, "c": 1, "v": {"v": 42.5, "m": "$42.50"}}));
        }
        let s = sheets(&doc, "wb").unwrap();
        assert_eq!(s[0].name, "Budget");
        assert_eq!(s[0].rows, vec![vec!["Item".to_string(), String::new()], vec![String::new(), "$42.50".into()]]);
    }
}
