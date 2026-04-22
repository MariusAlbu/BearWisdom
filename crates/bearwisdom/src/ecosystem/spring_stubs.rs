// =============================================================================
// ecosystem/spring_stubs.rs — Spring Framework synthetic stubs
//
// Spring's MockMvc test DSL is the single largest source of unresolved call
// refs in Kotlin / Java Spring projects. Two usage shapes both hit us:
//
//   Java-style static import:
//     import static org.springframework...MockMvcResultMatchers.jsonPath;
//     mockMvc.perform(get("/")).andExpect(status().isOk());
//
//   Kotlin DSL:
//     mockMvc.get("/api/books") .andExpect { jsonPath("$.books").value(...) }
//
// In the Kotlin DSL form `jsonPath` is a method on MockMvcResultMatchersDsl
// (the implicit receiver of the `andExpect` lambda). The Kotlin resolver's
// chain walker can't infer the lambda receiver's type, so the bare call
// drops through every import step and lands unresolved.
//
// Synthesise the MockMvc API as plain Function symbols. Combined with the
// Kotlin resolver's `by_name` fallback (step 7) the bare DSL calls resolve
// to their full qualified path. Pattern matches phoenix_stubs /
// laravel_stubs / swift_pm_dsl_stubs.
//
// Activation: Kotlin OR Java language present.
// =============================================================================

use std::path::Path;

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("spring-stubs");
const LEGACY_ECOSYSTEM_TAG: &str = "spring-stubs";
const LANGUAGES: &[&str] = &["kotlin", "java"];

// =============================================================================
// MockMvc inventories
// =============================================================================

const REQUEST_BUILDERS: &[&str] = &[
    "get", "post", "put", "delete", "patch", "options", "head",
    "fileUpload", "multipart", "request", "asyncDispatch",
];

const RESULT_MATCHERS: &[&str] = &[
    "status", "header", "content", "view", "model", "flash",
    "redirectedUrl", "redirectedUrlPattern",
    "forwardedUrl", "forwardedUrlPattern",
    "cookie", "jsonPath", "xpath", "handler",
];

const RESULT_HANDLERS: &[&str] = &["print", "log"];

const RESULT_ACTIONS: &[&str] = &["andExpect", "andDo", "andReturn", "andExpectAll"];

const STATUS_MATCHERS: &[&str] = &[
    "isOk", "isCreated", "isAccepted", "isNonAuthoritativeInformation",
    "isNoContent", "isResetContent", "isPartialContent", "isMultiStatus",
    "isAlreadyReported", "isImUsed",
    "isMultipleChoices", "isMovedPermanently", "isFound", "isSeeOther",
    "isNotModified", "isTemporaryRedirect", "isPermanentRedirect",
    "isBadRequest", "isUnauthorized", "isPaymentRequired", "isForbidden",
    "isNotFound", "isMethodNotAllowed", "isNotAcceptable",
    "isProxyAuthenticationRequired", "isRequestTimeout", "isConflict",
    "isGone", "isLengthRequired", "isPreconditionFailed",
    "isPayloadTooLarge", "isRequestEntityTooLarge", "isUriTooLong",
    "isRequestUriTooLong", "isUnsupportedMediaType",
    "isRequestedRangeNotSatisfiable", "isExpectationFailed",
    "isIAmATeapot", "isUnprocessableEntity", "isLocked",
    "isFailedDependency", "isTooEarly", "isUpgradeRequired",
    "isPreconditionRequired", "isTooManyRequests",
    "isRequestHeaderFieldsTooLarge", "isUnavailableForLegalReasons",
    "isInternalServerError", "isNotImplemented", "isBadGateway",
    "isServiceUnavailable", "isGatewayTimeout", "isHttpVersionNotSupported",
    "isVariantAlsoNegotiates", "isInsufficientStorage", "isLoopDetected",
    "isBandwidthLimitExceeded", "isNotExtended",
    "isNetworkAuthenticationRequired",
    "is1xxInformational", "is2xxSuccessful", "is3xxRedirection",
    "is4xxClientError", "is5xxServerError",
    "is", "reason",
];

const CONTENT_MATCHERS: &[&str] = &[
    "json", "xml", "html", "string", "bytes",
    "contentType", "contentTypeCompatibleWith", "encoding", "node", "source",
];

const HEADER_MATCHERS: &[&str] = &[
    "string", "exists", "doesNotExist", "longValue", "dateValue", "stringValues",
];

const JSON_PATH_MATCHERS: &[&str] = &[
    "value", "exists", "doesNotExist", "isEmpty", "isNotEmpty",
    "isArray", "isMap", "isBoolean", "isString", "isNumber",
    "hasJsonPath", "doesNotHaveJsonPath",
];

const COOKIE_MATCHERS: &[&str] = &[
    "value", "exists", "doesNotExist", "maxAge", "path",
    "domain", "secure", "httpOnly", "sameSite",
];

const XPATH_MATCHERS: &[&str] = &[
    "exists", "doesNotExist", "nodeCount", "string", "number", "booleanValue",
];

// =============================================================================
// Synthesis
// =============================================================================

fn sym(name: &str, qualified_name: &str, kind: SymbolKind, signature: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qualified_name.to_string(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some(signature.to_string()),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    }
}

fn synth_static_methods(
    class_qname: &'static str,
    names: &[&str],
    out_symbols: &mut Vec<ExtractedSymbol>,
) {
    out_symbols.push(sym(
        class_qname.rsplit('.').next().unwrap_or(class_qname),
        class_qname,
        SymbolKind::Namespace,
        &format!("class {class_qname}"),
    ));
    for name in names {
        out_symbols.push(sym(
            name,
            &format!("{class_qname}.{name}"),
            SymbolKind::Function,
            &format!("public static {name}(...)"),
        ));
    }
}

fn synthesize_file() -> ParsedFile {
    let mut symbols = Vec::new();

    synth_static_methods(
        "org.springframework.test.web.servlet.request.MockMvcRequestBuilders",
        REQUEST_BUILDERS,
        &mut symbols,
    );
    synth_static_methods(
        "org.springframework.test.web.servlet.result.MockMvcResultMatchers",
        RESULT_MATCHERS,
        &mut symbols,
    );
    synth_static_methods(
        "org.springframework.test.web.servlet.result.MockMvcResultHandlers",
        RESULT_HANDLERS,
        &mut symbols,
    );
    synth_static_methods(
        "org.springframework.test.web.servlet.ResultActions",
        RESULT_ACTIONS,
        &mut symbols,
    );
    synth_static_methods(
        "org.springframework.test.web.servlet.result.StatusResultMatchers",
        STATUS_MATCHERS,
        &mut symbols,
    );
    synth_static_methods(
        "org.springframework.test.web.servlet.result.ContentResultMatchers",
        CONTENT_MATCHERS,
        &mut symbols,
    );
    synth_static_methods(
        "org.springframework.test.web.servlet.result.HeaderResultMatchers",
        HEADER_MATCHERS,
        &mut symbols,
    );
    synth_static_methods(
        "org.springframework.test.web.servlet.result.JsonPathResultMatchers",
        JSON_PATH_MATCHERS,
        &mut symbols,
    );
    synth_static_methods(
        "org.springframework.test.web.servlet.result.CookieResultMatchers",
        COOKIE_MATCHERS,
        &mut symbols,
    );
    synth_static_methods(
        "org.springframework.test.web.servlet.result.XpathResultMatchers",
        XPATH_MATCHERS,
        &mut symbols,
    );

    let n_syms = symbols.len();
    ParsedFile {
        path: "ext:spring-stubs:MockMvc.kt".to_string(),
        language: "kotlin".to_string(),
        content_hash: format!("spring-stubs-{n_syms}"),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: vec![None; n_syms],
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: vec![false; n_syms],
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
    }
}

// =============================================================================
// Synthetic dep root + Ecosystem impl
// =============================================================================

fn synthetic_dep_root() -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: "spring-stubs".to_string(),
        version: String::new(),
        root: std::path::PathBuf::from("ext:spring-stubs"),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

pub struct SpringStubsEcosystem;

impl Ecosystem for SpringStubsEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::LanguagePresent("kotlin"),
            EcosystemActivation::LanguagePresent("java"),
        ])
    }

    fn locate_roots(&self, _ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        vec![synthetic_dep_root()]
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    fn uses_demand_driven_parse(&self) -> bool { true }

    fn parse_metadata_only(&self, _dep: &ExternalDepRoot) -> Option<Vec<ParsedFile>> {
        Some(vec![synthesize_file()])
    }
}

impl ExternalSourceLocator for SpringStubsEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }

    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        vec![synthetic_dep_root()]
    }

    fn parse_metadata_only(&self, _project_root: &Path) -> Option<Vec<ParsedFile>> {
        Some(vec![synthesize_file()])
    }
}

#[cfg(test)]
#[path = "spring_stubs_tests.rs"]
mod tests;
