use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use quote::ToTokens;
use serde::{Deserialize, Serialize};

const OLDEST_READER: &str = "0.2.4";
const OLDEST_SHAPE: &str = "tests/fixtures/wire-shape-0.2.4.json";
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

fn plain(fields: &[Field]) -> Vec<&Field> {
    fields.iter().filter(|f| !f.later).collect()
}

fn field_problem(old: &[Field], new: &[Field]) -> Option<String> {
    let (was, now) = (plain(old), plain(new));
    for i in 0..was.len().max(now.len()) {
        match (was.get(i), now.get(i)) {
            (Some(a), Some(b)) if a.ty == b.ty => continue,
            (Some(a), Some(b)) => return Some(format!("field {i} was `{}: {}`, now `{}: {}`", a.name, a.ty, b.name, b.ty)),
            (Some(a), None) => return Some(format!("field `{}: {}` is gone", a.name, a.ty)),
            (None, Some(b)) => return Some(format!("new field `{}: {}` is not a later field", b.name, b.ty)),
            (None, None) => break,
        }
    }
    None
}

fn shape_problem(name: &str, old: &Shape, new: &Shape) -> Option<String> {
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
                format!("new variants {added:?}: builds back to {OLDEST_READER} fail to read any value holding one")
            })
        }
        _ => Some("changed between struct and enum".into()),
    }
}

fn problems(old: &Shapes, new: &Shapes) -> Vec<String> {
    let mut found = Vec::new();
    for name in reachable(old) {
        let Some(new_defs) = new.get(&name) else {
            found.push(format!("{name}: gone"));
            continue;
        };
        for old_def in &old[&name] {
            let checks: Vec<Option<String>> = new_defs.iter().map(|new_def| shape_problem(&name, old_def, new_def)).collect();
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

fn oldest_shape() -> Shapes {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(OLDEST_SHAPE);
    serde_json::from_str(&std::fs::read_to_string(path).expect("the recorded shape exists")).expect("the recorded shape reads")
}

#[test]
#[ignore = "run once with WIRE_SHAPE_FROM=<checkout of the oldest supported release> to record its shape"]
fn record_the_oldest_supported_shape() {
    let from = std::env::var("WIRE_SHAPE_FROM").expect("WIRE_SHAPE_FROM names a checkout");
    let shapes = shapes_in(Path::new(&from));
    let kept: Shapes = reachable(&shapes).into_iter().map(|name| (name.clone(), shapes[&name].clone())).collect();
    let json = serde_json::to_string_pretty(&kept).unwrap();
    std::fs::write(Path::new(env!("CARGO_MANIFEST_DIR")).join(OLDEST_SHAPE), json + "\n").unwrap();
}

#[test]
fn builds_back_to_the_oldest_supported_version_read_what_this_build_writes() {
    let found = problems(&oldest_shape(), &shapes_in(&repo_root()));
    assert!(
        found.is_empty(),
        "Commy {OLDEST_READER} cannot read what this build writes. Put new fields at the end with \
         #[serde(default, with = \"crate::wire_layout::later\")], never reorder, retype or remove fields, and only add \
         variants to {ENUMS_OLD_READERS_SKIP_WHOLE:?}:\n{}",
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
    assert!(problems(&old, &later).is_empty());
    let mut plain = sample();
    edit(&mut plain, "Payload", |s| push_field(s, false));
    assert_eq!(problems(&old, &plain).len(), 1);
}

#[test]
fn retyped_reordered_or_removed_fields_are_caught() {
    let old = sample();
    let mut retyped = sample();
    edit(&mut retyped, "Payload", |s| {
        let Shape::Struct { fields, .. } = s else { panic!("a struct") };
        fields[0].ty = "u64".into();
    });
    assert_eq!(problems(&old, &retyped).len(), 1);
    let mut swapped = sample();
    edit(&mut swapped, "Payload", |s| {
        let Shape::Struct { fields, .. } = s else { panic!("a struct") };
        fields.swap(0, 1);
    });
    assert_eq!(problems(&old, &swapped).len(), 1);
    let mut gone = sample();
    gone.remove("Kind");
    assert_eq!(problems(&old, &gone), vec!["Kind: gone".to_string()]);
}

#[test]
fn new_variants_are_caught_unless_old_readers_skip_the_whole_value() {
    let old = sample();
    let mut added = sample();
    edit(&mut added, "Kind", |s| {
        let Shape::Enum { variants, .. } = s else { panic!("an enum") };
        variants.push(Variant { name: "Forum".into(), fields: Vec::new() });
    });
    assert_eq!(problems(&old, &added).len(), 1);
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
    assert!(problems(&renamed_old, &renamed_new).is_empty());
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
