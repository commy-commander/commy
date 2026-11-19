use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use quote::ToTokens;
use serde::{Deserialize, Serialize};

const FIXTURES: &str = "tests/fixtures";
const SHAPE_PREFIX: &str = "wire-shape-";
const PEER_PROTOCOL: (&str, &str) = ("crates/commy-net/src/protocol.rs", "PROTOCOL_VERSION");
const SYNC_PROTOCOL: (&str, &str) = ("crates/commy-sync/src/wire.rs", "PROTOCOL");
const OLDEST_SYNC_CLIENT: (&str, &str) = ("crates/commy-sync/src/wire.rs", "OLDEST_CLIENT_PROTOCOL");
const SOURCES: [&str; 2] = ["crates/commy-core/src", "crates/commy-net/src"];
const ROOTS: [&str; 6] = ["Envelope", "Entry", "SignalEntry", "DmEnvelope", "CatchupFrame", "SignedBytes"];
const ENUMS_OLD_READERS_SKIP_WHOLE: [&str; 3] = ["ServerEvent", "WirePayload", "CatchupFrame"];
const LATER: &str = "wire_layout::later";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Field {
    name: String,
    ty: String,
    later: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Variant {
    name: String,
    fields: Vec<Field>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Shape {
    Struct { file: String, fields: Vec<Field> },
    Enum { file: String, variants: Vec<Variant> },
}

type Shapes = BTreeMap<String, Vec<Shape>>;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct Recorded {
    constants: BTreeMap<String, u64>,
    types: Shapes,
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

fn tokens(item: &impl ToTokens) -> String {
    item.to_token_stream().to_string().split_whitespace().collect()
}

fn derives_serialize(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident("derive") && tokens(attr).contains("Serialize"))
}

fn is_test_only(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident("cfg") && tokens(attr).contains("test"))
}

fn serde_attrs(attrs: &[syn::Attribute]) -> String {
    attrs.iter().filter(|attr| attr.path().is_ident("serde")).map(tokens).collect::<Vec<_>>().join("")
}

fn skipped(serde: &str) -> bool {
    ["skip)", "skip,", "skip_serializing", "skip_deserializing"].iter().any(|skip| serde.contains(skip))
}

fn without_paths(ty: &str) -> String {
    let mut out = String::new();
    let mut word = String::new();
    let mut chars = ty.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_alphanumeric() || c == '_' {
            word.push(c);
            continue;
        }
        if c == ':' && chars.peek() == Some(&':') {
            chars.next();
            word.clear();
            continue;
        }
        out.push_str(&word);
        word.clear();
        out.push(c);
    }
    out + &word
}

fn custom_with(serde: &str) -> String {
    serde
        .split("with=")
        .nth(1)
        .and_then(|rest| rest.split('"').nth(1))
        .filter(|with| !with.ends_with(LATER))
        .map(|with| format!(" with {}", without_paths(with)))
        .unwrap_or_default()
}

fn fields_of(fields: &syn::Fields) -> Vec<Field> {
    fields
        .iter()
        .enumerate()
        .filter_map(|(i, field)| {
            let serde = serde_attrs(&field.attrs);
            if skipped(&serde) {
                return None;
            }
            let name = field.ident.as_ref().map_or_else(|| i.to_string(), ToString::to_string);
            let ty = without_paths(&tokens(&field.ty)) + &custom_with(&serde);
            Some(Field { name, ty, later: serde.contains(LATER) })
        })
        .collect()
}

fn collect(items: &[syn::Item], file: &str, shapes: &mut Shapes) {
    for item in items {
        match item {
            syn::Item::Struct(s) if derives_serialize(&s.attrs) => {
                let shape = Shape::Struct { file: file.into(), fields: fields_of(&s.fields) };
                shapes.entry(s.ident.to_string()).or_default().push(shape);
            }
            syn::Item::Enum(e) if derives_serialize(&e.attrs) => {
                let variants = e
                    .variants
                    .iter()
                    .filter(|v| !skipped(&serde_attrs(&v.attrs)))
                    .map(|v| Variant { name: v.ident.to_string(), fields: fields_of(&v.fields) })
                    .collect();
                shapes.entry(e.ident.to_string()).or_default().push(Shape::Enum { file: file.into(), variants });
            }
            syn::Item::Mod(m) if !is_test_only(&m.attrs) => {
                if let Some((_, inner)) = &m.content {
                    collect(inner, file, shapes);
                }
            }
            _ => {}
        }
    }
}

fn shapes_in(root: &Path) -> Shapes {
    let mut files = Vec::new();
    for dir in SOURCES {
        rust_files(&root.join(dir), &mut files);
    }
    files.sort();
    let mut shapes = Shapes::new();
    for path in files {
        let text = std::fs::read_to_string(&path).expect("source reads");
        let parsed = syn::parse_file(&text).unwrap_or_else(|e| panic!("{} parses: {e}", path.display()));
        let file = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
        collect(&parsed.items, &file, &mut shapes);
    }
    shapes
}

fn type_names(ty: &str) -> impl Iterator<Item = &str> {
    ty.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|word| word.starts_with(|c: char| c.is_ascii_uppercase()))
}

fn fields_of_shape(shape: &Shape) -> Vec<&Field> {
    match shape {
        Shape::Struct { fields, .. } => fields.iter().collect(),
        Shape::Enum { variants, .. } => variants.iter().flat_map(|v| &v.fields).collect(),
    }
}

fn reachable(shapes: &Shapes) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut next: Vec<String> = ROOTS.iter().map(|root| root.to_string()).collect();
    while let Some(name) = next.pop() {
        let Some(defs) = shapes.get(&name) else { continue };
        if !seen.insert(name) {
            continue;
        }
        for field in defs.iter().flat_map(fields_of_shape) {
            next.extend(type_names(&field.ty).filter(|n| shapes.contains_key(*n)).map(str::to_string));
        }
    }
    seen
}

fn field_problem(old: &[Field], new: &[Field]) -> Option<String> {
    for (i, was) in old.iter().enumerate() {
        match new.get(i) {
            Some(now) if now.ty == was.ty && now.later == was.later => continue,
            Some(now) => return Some(format!("field {i} was `{}: {}`, now `{}: {}`", was.name, was.ty, now.name, now.ty)),
            None => return Some(format!("field `{}: {}` is gone", was.name, was.ty)),
        }
    }
    new.iter()
        .skip(old.len())
        .find(|now| !now.later)
        .map(|now| format!("new field `{}: {}` is not a later field", now.name, now.ty))
}

fn shape_problem(name: &str, old: &Shape, new: &Shape, release: &str) -> Option<String> {
    match (old, new) {
        (Shape::Struct { fields: was, .. }, Shape::Struct { fields: now, .. }) => field_problem(was, now),
        (Shape::Enum { variants: was, .. }, Shape::Enum { variants: now, .. }) => {
            for (i, old_variant) in was.iter().enumerate() {
                let Some(new_variant) = now.get(i) else {
                    return Some(format!("variant {i} ({}) is gone", old_variant.name));
                };
                if let Some(problem) = field_problem(&old_variant.fields, &new_variant.fields) {
                    return Some(format!("variant {i} ({}): {problem}", new_variant.name));
                }
            }
            let added: Vec<&str> = now.iter().skip(was.len()).map(|v| v.name.as_str()).collect();
            (!added.is_empty() && !ENUMS_OLD_READERS_SKIP_WHOLE.contains(&name)).then(|| {
                format!("new variants {added:?}: Commy {release} fails to read any value holding one")
            })
        }
        _ => Some("changed between struct and enum".into()),
    }
}

fn problems(old: &Shapes, new: &Shapes, release: &str) -> Vec<String> {
    let mut found = Vec::new();
    for name in reachable(old) {
        let Some(new_defs) = new.get(&name) else {
            found.push(format!("{name}: gone"));
            continue;
        };
        for old_def in &old[&name] {
            let checks: Vec<Option<String>> = new_defs.iter().map(|new_def| shape_problem(&name, old_def, new_def, release)).collect();
            if checks.iter().any(Option::is_none) {
                continue;
            }
            if let Some(problem) = checks.into_iter().flatten().next() {
                found.push(format!("{name}: {problem}"));
            }
        }
    }
    found
}

fn constant(root: &Path, (file, name): (&str, &str)) -> Option<u64> {
    let text = std::fs::read_to_string(root.join(file)).unwrap_or_else(|e| panic!("{file} reads: {e}"));
    let parsed = syn::parse_file(&text).unwrap_or_else(|e| panic!("{file} parses: {e}"));
    parsed
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Const(c) if c.ident == name => match &*c.expr {
                syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Int(int), .. }) => int.base10_parse().ok(),
                _ => None,
            },
            _ => None,
        })
}

fn constant_key((file, name): (&str, &str)) -> String {
    format!("{file}::{name}")
}

fn record(root: &Path) -> Recorded {
    let shapes = shapes_in(root);
    let types = reachable(&shapes).into_iter().map(|name| (name.clone(), shapes[&name].clone())).collect();
    let constants = [PEER_PROTOCOL, SYNC_PROTOCOL, OLDEST_SYNC_CLIENT].into_iter().filter_map(|c| Some((constant_key(c), constant(root, c)?))).collect();
    Recorded { constants, types }
}

fn protocol_problems(old: &Recorded, new: &Recorded, release: &str) -> Vec<String> {
    let get = |recorded: &Recorded, c| recorded.constants.get(&constant_key(c)).copied().unwrap_or_default();
    let mut found = Vec::new();
    if get(new, PEER_PROTOCOL) != get(old, PEER_PROTOCOL) {
        found.push(format!(
            "{} changed from {} to {}: Commy {release} drops every message and event from a different version",
            PEER_PROTOCOL.1,
            get(old, PEER_PROTOCOL),
            get(new, PEER_PROTOCOL)
        ));
    }
    if get(new, OLDEST_SYNC_CLIENT) > get(old, SYNC_PROTOCOL) {
        found.push(format!(
            "{} is {}, above the sync protocol {} that Commy {release} speaks, so sync servers on this build turn it away",
            OLDEST_SYNC_CLIENT.1,
            get(new, OLDEST_SYNC_CLIENT),
            get(old, SYNC_PROTOCOL)
        ));
    }
    found
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURES)
}

fn recorded_releases() -> Vec<(String, Recorded)> {
    let mut releases: Vec<(String, Recorded)> = std::fs::read_dir(fixtures_dir())
        .expect("the fixtures folder exists")
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let release = name.strip_prefix(SHAPE_PREFIX)?.strip_suffix(".json")?.to_string();
            let text = std::fs::read_to_string(entry.path()).ok()?;
            Some((release, serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name} reads: {e}"))))
        })
        .collect();
    releases.sort_by(|a, b| a.0.cmp(&b.0));
    releases
}

#[test]
#[ignore = "run at each final release with WIRE_SHAPE_FROM=<checkout of the tag> and WIRE_SHAPE_RELEASE=<version>"]
fn record_a_stable_release() {
    let from = std::env::var("WIRE_SHAPE_FROM").expect("WIRE_SHAPE_FROM names a checkout");
    let release = std::env::var("WIRE_SHAPE_RELEASE").expect("WIRE_SHAPE_RELEASE names the version");
    let json = serde_json::to_string_pretty(&record(Path::new(&from))).unwrap();
    std::fs::write(fixtures_dir().join(format!("{SHAPE_PREFIX}{release}.json")), json + "\n").unwrap();
}

#[test]
fn every_recorded_stable_release_reads_what_this_build_writes() {
    let releases = recorded_releases();
    assert!(!releases.is_empty(), "at least one stable release is recorded");
    let now = record(&repo_root());
    let found: Vec<String> = releases
        .iter()
        .flat_map(|(release, old)| {
            let mut found = protocol_problems(old, &now, release);
            found.extend(problems(&old.types, &now.types, release));
            found.into_iter().map(move |problem| format!("[{release}] {problem}"))
        })
        .collect();
    assert!(
        found.is_empty(),
        "Stable Commy releases cannot read what this build writes, so people on it would stop seeing each other. \
         Put new fields at the end with #[serde(default, with = \"crate::wire_layout::later\")], never reorder, retype or \
         remove fields, only add variants to {ENUMS_OLD_READERS_SKIP_WHOLE:?}, and keep the protocol versions:\n{}",
        found.join("\n")
    );
}

fn sample() -> Shapes {
    let field = |name: &str, ty: &str, later: bool| Field { name: name.into(), ty: ty.into(), later };
    let mut shapes = Shapes::new();
    shapes.insert("Envelope".into(), vec![Shape::Struct { file: "a.rs".into(), fields: vec![field("payload", "Payload", false)] }]);
    shapes.insert(
        "Payload".into(),
        vec![Shape::Struct { file: "a.rs".into(), fields: vec![field("name", "String", false), field("kind", "Kind", false)] }],
    );
    shapes.insert(
        "Kind".into(),
        vec![Shape::Enum { file: "a.rs".into(), variants: vec![Variant { name: "Text".into(), fields: Vec::new() }] }],
    );
    shapes
}

fn edit(shapes: &mut Shapes, name: &str, change: impl FnOnce(&mut Shape)) {
    change(&mut shapes.get_mut(name).unwrap()[0]);
}

fn push_field(shape: &mut Shape, later: bool) {
    let Shape::Struct { fields, .. } = shape else { panic!("a struct") };
    fields.push(Field { name: "home".into(), ty: "Option<String>".into(), later });
}

#[test]
fn a_new_field_must_be_a_later_field() {
    let old = sample();
    let mut later = sample();
    edit(&mut later, "Payload", |s| push_field(s, true));
    assert!(problems(&old, &later, "0.2.4").is_empty());
    let mut plain = sample();
    edit(&mut plain, "Payload", |s| push_field(s, false));
    assert_eq!(problems(&old, &plain, "0.2.4").len(), 1);
}

#[test]
fn retyped_reordered_or_removed_fields_are_caught() {
    let old = sample();
    let mut retyped = sample();
    edit(&mut retyped, "Payload", |s| {
        let Shape::Struct { fields, .. } = s else { panic!("a struct") };
        fields[0].ty = "u64".into();
    });
    assert_eq!(problems(&old, &retyped, "0.2.4").len(), 1);
    let mut swapped = sample();
    edit(&mut swapped, "Payload", |s| {
        let Shape::Struct { fields, .. } = s else { panic!("a struct") };
        fields.swap(0, 1);
    });
    assert_eq!(problems(&old, &swapped, "0.2.4").len(), 1);
    let mut gone = sample();
    gone.remove("Kind");
    assert_eq!(problems(&old, &gone, "0.2.4"), vec!["Kind: gone".to_string()]);
}

#[test]
fn new_variants_are_caught_unless_old_readers_skip_the_whole_value() {
    let old = sample();
    let mut added = sample();
    edit(&mut added, "Kind", |s| {
        let Shape::Enum { variants, .. } = s else { panic!("an enum") };
        variants.push(Variant { name: "Forum".into(), fields: Vec::new() });
    });
    assert_eq!(problems(&old, &added, "0.2.4").len(), 1);
    let mut renamed_old = old.clone();
    let kind = renamed_old.remove("Kind").unwrap();
    renamed_old.insert("ServerEvent".into(), kind);
    edit(&mut renamed_old, "Payload", |s| {
        let Shape::Struct { fields, .. } = s else { panic!("a struct") };
        fields[1].ty = "ServerEvent".into();
    });
    let mut renamed_new = added.clone();
    let kind = renamed_new.remove("Kind").unwrap();
    renamed_new.insert("ServerEvent".into(), kind);
    edit(&mut renamed_new, "Payload", |s| {
        let Shape::Struct { fields, .. } = s else { panic!("a struct") };
        fields[1].ty = "ServerEvent".into();
    });
    assert!(problems(&renamed_old, &renamed_new, "0.2.4").is_empty());
}

#[test]
fn types_read_from_source_with_paths_and_later_fields() {
    let parsed = syn::parse_file(
        r#"
        #[derive(Serialize, Deserialize)]
        pub struct Server {
            pub id: commy_core::ServerId,
            #[serde(skip)]
            cache: u8,
            #[serde(default, with = "crate::wire_layout::later")]
            pub home: Option<String>,
        }
        #[cfg(test)]
        mod tests {
            #[derive(Serialize)]
            struct Hidden;
        }
        "#,
    )
    .unwrap();
    let mut shapes = Shapes::new();
    collect(&parsed.items, "x.rs", &mut shapes);
    assert_eq!(shapes.keys().collect::<Vec<_>>(), vec!["Server"]);
    let Shape::Struct { fields, .. } = &shapes["Server"][0] else { panic!("a struct") };
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].ty, "ServerId");
    assert!(fields[1].later);
}

#[test]
fn a_later_field_keeps_its_type_once_a_stable_release_has_it() {
    let field = |ty: &str, later: bool| Field { name: "home".into(), ty: ty.into(), later };
    assert!(field_problem(&[field("Option<String>", true)], &[field("Option<String>", true)]).is_none());
    assert!(field_problem(&[field("Option<String>", true)], &[field("Vec<String>", true)]).is_some());
    assert!(field_problem(&[field("Option<String>", true)], &[field("Option<String>", false)]).is_some());
}

#[test]
fn a_different_peer_protocol_or_a_raised_oldest_sync_client_is_caught() {
    let recorded = |peer: u64, sync: u64, oldest: u64| Recorded {
        constants: [(constant_key(PEER_PROTOCOL), peer), (constant_key(SYNC_PROTOCOL), sync), (constant_key(OLDEST_SYNC_CLIENT), oldest)].into(),
        types: Shapes::new(),
    };
    let stable = recorded(4, 7, 1);
    assert!(protocol_problems(&stable, &recorded(4, 9, 1), "1.0.0").is_empty());
    assert!(protocol_problems(&stable, &recorded(4, 9, 7), "1.0.0").is_empty());
    assert_eq!(protocol_problems(&stable, &recorded(5, 7, 1), "1.0.0").len(), 1);
    assert_eq!(protocol_problems(&stable, &recorded(4, 9, 8), "1.0.0").len(), 1);
}

#[test]
fn this_checkout_reads_its_own_protocol_versions() {
    let now = record(&repo_root());
    assert!(now.constants[&constant_key(PEER_PROTOCOL)] > 0);
    assert!(now.constants[&constant_key(SYNC_PROTOCOL)] >= now.constants[&constant_key(OLDEST_SYNC_CLIENT)]);
}
