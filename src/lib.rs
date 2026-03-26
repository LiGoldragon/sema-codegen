mod error;

pub use error::Error;

/// Schema files embedded at compile time from flake-crates/.
const SEMA_CORE_INIT: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/flake-crates/sema-core/sema-core-init.cozo"));
const SEMA_CORE_SEED: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/flake-crates/sema-core/sema-core-seed.cozo"));
const SEMA_INIT: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/flake-crates/sema/sema-lang-init.cozo"));
const SEMA_SEED: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/flake-crates/sema/sema-lang-seed.cozo"));

pub struct EnumSchema {
    pub name: String,
    pub variants: Vec<String>,
}

pub struct SchemaGenerator {
    pub enums: Vec<EnumSchema>,
}

impl SchemaGenerator {
    /// Boot an in-memory CozoDB from embedded .cozo files, discover all
    /// triad-shaped domains, and build enum schemas in ordinal order.
    pub fn from_embedded() -> Result<Self, Error> {
        let db = boot_db()?;
        Self::from_db(&db)
    }

    /// Build from an already-loaded CozoDB instance.
    pub fn from_db(db: &criome_cozo::CriomeDb) -> Result<Self, Error> {
        let domains = discover_triad_domains(db)?;
        let mut enums = Vec::new();

        for (name, key_col, variant_col) in &domains {
            let variants = query_ordered_variants(db, name, key_col, variant_col)?;
            if !variants.is_empty() {
                enums.push(EnumSchema {
                    name: name.clone(),
                    variants,
                });
            }
        }

        Ok(Self { enums })
    }

    pub fn to_capnp_text(&self) -> String {
        let file_id = self.file_id();
        let mut out = format!("@0x{file_id:016x};\n\n");

        let mut sorted: Vec<&EnumSchema> = self.enums.iter().collect();
        sorted.sort_by(|a, b| a.name.cmp(&b.name));

        for e in &sorted {
            out.push_str(&format!("enum {} {{\n", e.name));
            for (i, variant) in e.variants.iter().enumerate() {
                out.push_str(&format!("  {} @{};\n", to_capnp_enumerant(variant), i));
            }
            out.push_str("}\n\n");
        }

        out
    }

    pub fn schema_hash(&self) -> blake3::Hash {
        blake3::hash(self.to_capnp_text().as_bytes())
    }

    fn file_id(&self) -> u64 {
        let mut hasher = blake3::Hasher::new();
        for e in &self.enums {
            hasher.update(e.name.as_bytes());
        }
        let hash = hasher.finalize();
        let bytes: [u8; 8] = hash.as_bytes()[..8].try_into().unwrap();
        u64::from_le_bytes(bytes) | 0x8000000000000000
    }
}

/// Boot an in-memory CozoDB and load all embedded .cozo schemas.
fn boot_db() -> Result<criome_cozo::CriomeDb, Error> {
    let db = criome_cozo::CriomeDb::open_memory()
        .map_err(|e| Error::Schema(format!("open_memory: {e}")))?;

    let scripts: &[(&str, &str)] = &[
        ("sema-core-init", SEMA_CORE_INIT),
        ("sema-core-seed", SEMA_CORE_SEED),
        ("sema-init", SEMA_INIT),
        ("sema-seed", SEMA_SEED),
    ];

    for (label, source) in scripts {
        for stmt in criome_cozo::Script::from_str(source) {
            let trimmed = stmt.trim();
            if !trimmed.is_empty() && !trimmed.lines().all(|l| l.trim().starts_with('#')) {
                db.run_script(trimmed)
                    .map_err(|e| Error::Schema(format!("{label}: {e}")))?;
            }
        }
    }

    Ok(db)
}

/// Discover all triad-shaped domains: relations with exactly one Int key
/// and String value columns. Returns (relation_name, key_col_name, variant_col_name).
fn discover_triad_domains(db: &criome_cozo::CriomeDb) -> Result<Vec<(String, String, String)>, Error> {
    let rels = db.run_script("::relations")
        .map_err(|e| Error::Query(format!("::relations: {e}")))?;
    let rel_rows = rels.get("rows").and_then(|v| v.as_array())
        .ok_or_else(|| Error::Schema("::relations missing rows".into()))?;

    let mut triads = Vec::new();

    for row in rel_rows {
        let name = match row.as_array().and_then(|a| a.first()).and_then(|v| v.as_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        // Skip lowercase relations (not domains)
        if name.chars().next().is_none_or(|c| c.is_lowercase()) {
            continue;
        }

        let cols = db.run_script(&format!("::columns {name}"));
        let cols = match cols {
            Ok(c) => c,
            Err(_) => continue,
        };
        let col_rows = match cols.get("rows").and_then(|v| v.as_array()) {
            Some(r) => r,
            None => continue,
        };

        // Parse column info: (name, is_key, type)
        let mut int_key = None;
        let mut string_vals = Vec::new();

        for col in col_rows {
            let arr = match col.as_array() {
                Some(a) => a,
                None => continue,
            };
            let col_name = arr.get(1).and_then(|v| v.as_str()).unwrap_or_default();
            let is_key = arr.get(2).and_then(|v| v.as_bool()).unwrap_or(false);
            let col_type = arr.get(3).and_then(|v| v.as_str()).unwrap_or_default();

            if is_key && col_type == "Int" {
                int_key = Some(col_name.to_string());
            } else if !is_key && col_type == "String" {
                string_vals.push(col_name.to_string());
            }
        }

        // Triad: exactly one Int key + at least one String value (the variant name)
        if let Some(key_col) = int_key {
            if let Some(variant_col) = string_vals.first() {
                triads.push((name, key_col, variant_col.clone()));
            }
        }
    }

    triads.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(triads)
}

/// Query variants from a triad-shaped domain, ordered by the Int key.
fn query_ordered_variants(
    db: &criome_cozo::CriomeDb,
    domain: &str,
    key_col: &str,
    variant_col: &str,
) -> Result<Vec<String>, Error> {
    let query = format!(
        "?[{variant_col}] := *{domain}{{{key_col}, {variant_col}}} :order {key_col}"
    );
    let result = db.run_script(&query)
        .map_err(|e| Error::Query(format!("{domain}: {e}")))?;

    let rows = result.get("rows").and_then(|v| v.as_array())
        .ok_or_else(|| Error::Query(format!("{domain}: missing rows")))?;

    Ok(rows
        .iter()
        .filter_map(|row| row.as_array()?.first()?.as_str().map(String::from))
        .collect())
}

fn to_capnp_enumerant(s: &str) -> String {
    if s.contains('_') || s.contains('-') {
        to_camel_case(s)
    } else {
        let mut chars = s.chars();
        match chars.next() {
            None => String::new(),
            Some(c) => {
                let lower: String = c.to_lowercase().collect();
                lower + chars.as_str()
            }
        }
    }
}

fn to_camel_case(s: &str) -> String {
    let parts: Vec<&str> = s.split(|c| c == '_' || c == '-').collect();
    let mut result = String::new();
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            result.push_str(part);
        } else {
            let mut chars = part.chars();
            if let Some(c) = chars.next() {
                result.push_str(&c.to_uppercase().to_string());
                result.push_str(chars.as_str());
            }
        }
    }
    result
}
