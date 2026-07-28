use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const APPROVED_CAPABILITY_MINT_POINTS: &[&str] =
    &["router::ZoneRegistrar::method:component_session_acceptor"];
const APPROVED_CAPABILITY_PUBLIC_API: &[&str] = &[
    "router::ComponentSessionAdmission",
    "router::ZoneRegistrar::method:component_session_acceptor",
    "router::ZoneRegistrar::method:reconnect_component_session",
    "router::ZoneRegistrar::method:register_component_session",
];

#[test]
fn public_api_has_only_the_approved_capability_mint_surface() {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repository_root = crate_root.parent().unwrap().parent().unwrap();
    let scratch = repository_root
        .join(".scratch")
        .join(format!("bus-public-api-{}", std::process::id()));
    let temp = scratch.join("tmp");
    fs::create_dir_all(&temp).expect("create repository-local rustdoc scratch");

    let output = Command::new(env!("CARGO"))
        .args([
            "doc",
            "--quiet",
            "--locked",
            "--no-deps",
            "--manifest-path",
            crate_root.join("Cargo.toml").to_str().unwrap(),
            "--target-dir",
            scratch.join("target").to_str().unwrap(),
        ])
        .env("TMPDIR", &temp)
        .output()
        .expect("render the compiler-owned public API");
    assert!(
        output.status.success(),
        "rustdoc failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let (actual, capability_surface) = public_api(&scratch.join("target/doc/d2b_bus"));
    let approved = include_str!("approved-public-api.txt")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actual, approved,
        "d2b-bus public API changed. The only approved capability mint point is \
         {APPROVED_CAPABILITY_MINT_POINTS:?}. Review every API delta for a new \
         constructor, factory, capability accessor, or externally implementable \
         producer before updating tests/approved-public-api.txt."
    );
    for mint in APPROVED_CAPABILITY_MINT_POINTS {
        assert!(
            actual.contains(*mint),
            "approved capability mint point {mint:?} is absent from the actual public API"
        );
    }
    assert_eq!(
        capability_surface,
        APPROVED_CAPABILITY_PUBLIC_API
            .iter()
            .copied()
            .map(str::to_owned)
            .collect(),
        "a public signature now exposes a sealed capability outside the explicitly \
         approved capability API {APPROVED_CAPABILITY_PUBLIC_API:?}"
    );

    let router = fs::read_to_string(crate_root.join("src/router.rs")).expect("read router source");
    assert_eq!(
        source_occurrences(&router, "\n            ComponentSessionAdmission {"),
        1,
        "ComponentSessionAdmission must be constructed only by the approved registrar mint point"
    );
    assert_eq!(
        source_occurrences(&router, "SessionAcceptor::new("),
        1,
        "SessionAcceptor construction widened beyond the approved registrar mint point"
    );

    fs::remove_dir_all(&scratch).expect("remove repository-local rustdoc scratch");
}

fn source_occurrences(source: &str, needle: &str) -> usize {
    source.match_indices(needle).count()
}

fn public_api(doc_root: &Path) -> (BTreeSet<String>, BTreeSet<String>) {
    let all = fs::read_to_string(doc_root.join("all.html")).expect("read rustdoc all-items page");
    let mut public = BTreeSet::new();
    let mut capability_surface = BTreeSet::new();
    for entry in all.split("<li><a href=\"").skip(1) {
        let Some((href, rest)) = entry.split_once('"') else {
            continue;
        };
        if href.starts_with('#') || !href.ends_with(".html") {
            continue;
        }
        let Some((_, text)) = rest.split_once('>') else {
            continue;
        };
        let Some((name, _)) = text.split_once("</a>") else {
            continue;
        };
        public.insert(name.to_owned());

        let item = doc_root.join(href);
        if item.is_file() {
            let html = fs::read_to_string(item).expect("read rustdoc item page");
            if item_declaration(&html).is_some_and(is_capability_signature) {
                capability_surface.insert(name.to_owned());
            }
            collect_members(name, &html, &mut public, &mut capability_surface);
        }
    }
    (public, capability_surface)
}

fn collect_members(
    item: &str,
    html: &str,
    public: &mut BTreeSet<String>,
    capability_surface: &mut BTreeSet<String>,
) {
    for section in html.split("<section id=\"").skip(1) {
        let Some((id, rest)) = section.split_once('"') else {
            continue;
        };
        let Some((kind, member)) = id.split_once('.') else {
            continue;
        };
        if !matches!(
            kind,
            "method" | "tymethod" | "structfield" | "associatedtype" | "associatedconstant"
        ) {
            continue;
        }
        let Some((class_prefix, body)) = rest.split_once('>') else {
            continue;
        };
        let body = body.split_once("</section>").map_or(body, |(body, _)| body);
        if class_prefix.contains("trait-impl") {
            continue;
        }
        if kind == "method" && !body.contains("<h4 class=\"code-header\">pub ") {
            continue;
        }
        let symbol = format!("{item}::{kind}:{member}");
        if code_header(body).is_some_and(is_capability_signature) {
            capability_surface.insert(symbol.clone());
        }
        public.insert(symbol);
    }
}

fn item_declaration(html: &str) -> Option<&str> {
    html.split_once("<pre class=\"rust item-decl\"><code>")?
        .1
        .split_once("</code></pre>")
        .map(|(declaration, _)| declaration)
}

fn code_header(section: &str) -> Option<&str> {
    section
        .split_once("<h4 class=\"code-header\">")?
        .1
        .split_once("</h4>")
        .map(|(header, _)| header)
}

fn is_capability_signature(signature: &str) -> bool {
    [
        "AuthenticatedComponentSession",
        "ComponentSessionAdmission",
        "SessionAcceptor",
        "SessionAuthority",
        "SessionRegistration",
        "SessionRegistrationCapability",
    ]
    .iter()
    .any(|marker| signature.contains(marker))
}
