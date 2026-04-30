// =============================================================================
// ecosystem/jest_synthetics.rs — Jest / Vitest test framework synthetic stubs
//
// Test-runner globals (`describe`, `it`, `expect`, `beforeEach`, …) are the
// largest single contributor to TypeScript/JavaScript unresolved refs in
// projects that don't have `@types/jest` installed. The Jest API also ships
// `jest.Mock` / `jest.Mocked` / `jest.MockedFunction` namespace types that
// appear in test signatures and have no real declaration unless `@types/jest`
// is on disk.
//
// Mirroring `phoenix_stubs` / `spring_stubs` / `laravel_stubs` we synthesise
// the public Jest surface as plain `Function` / `Variable` / `Class` symbols
// under the index root. The TS/JS resolver's `by_name` step picks them up
// and the `jest.*` namespace lookups resolve via `by_qualified_name`.
//
// Vitest (`vi.*` namespace and the same `describe`/`it`/`expect` globals)
// shares 90% of the surface — we emit a `vi`-prefixed mirror of the jest
// namespace so the same activation covers both.
//
// Activation: TypeScript / JavaScript / Vue / Svelte / Astro / Angular
// language present. Synthetic symbols sit harmless when the project doesn't
// actually use a test runner.
// =============================================================================

use std::path::Path;

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("jest-synthetics");
const LEGACY_ECOSYSTEM_TAG: &str = "jest-synthetics";
const LANGUAGES: &[&str] = &[
    "typescript", "tsx", "javascript", "jsx",
    "vue", "svelte", "astro", "angular",
];

// =============================================================================
// Inventories — Jest (and overlapping Mocha / Vitest globals)
// =============================================================================

/// Bare-name globals injected into every test file by jest, vitest, mocha,
/// jasmine, ava, and most other JS test runners. Always-on regardless of
/// import.
const TEST_GLOBALS: &[&str] = &[
    "describe",
    "it",
    "test",
    "expect",
    "beforeEach",
    "afterEach",
    "beforeAll",
    "afterAll",
    "before",
    "after",
    // Variants for selective and skip
    "fdescribe",
    "fit",
    "xdescribe",
    "xit",
    "xtest",
    "skip",
    "only",
    "todo",
    "concurrent",
    // Hooks variants
    "setup",
    "teardown",
    // Assertion helpers (chai-style root globals when chai is auto-imported
    // via mocha config; resolves chained `should()`/`expect(x).to.equal(y)`
    // root names without bringing chai into a separate ecosystem).
    "assert",
    "should",
];

/// `jest.*` namespace surface — types and helper functions accessible as
/// dotted refs in test files (`jest.fn()`, `jest.Mock`, `jest.MockedFunction`).
const JEST_NAMESPACE_MEMBERS: &[(&str, SymbolKind, &str)] = &[
    // Type aliases / generic interfaces
    ("Mock", SymbolKind::Interface, "interface Mock<T = any, Y extends any[] = any[]>"),
    ("Mocked", SymbolKind::TypeAlias, "type Mocked<T>"),
    ("MockedFunction", SymbolKind::TypeAlias, "type MockedFunction<T extends (...args: any) => any>"),
    ("MockedClass", SymbolKind::TypeAlias, "type MockedClass<T>"),
    ("MockedObject", SymbolKind::TypeAlias, "type MockedObject<T>"),
    ("SpyInstance", SymbolKind::Interface, "interface SpyInstance<T = any, Y extends any[] = any[]>"),
    ("SpiedFunction", SymbolKind::TypeAlias, "type SpiedFunction<T>"),
    ("It", SymbolKind::Interface, "interface It"),
    ("Describe", SymbolKind::Interface, "interface Describe"),
    ("Lifecycle", SymbolKind::Interface, "interface Lifecycle"),
    ("Expect", SymbolKind::Interface, "interface Expect"),
    ("Matchers", SymbolKind::Interface, "interface Matchers<R>"),
    ("AsymmetricMatcher", SymbolKind::Interface, "interface AsymmetricMatcher"),
    ("CustomMatcher", SymbolKind::TypeAlias, "type CustomMatcher"),
    ("DoneCallback", SymbolKind::TypeAlias, "type DoneCallback = (...args: any[]) => void"),
    ("ProvidesHookCallback", SymbolKind::TypeAlias, "type ProvidesHookCallback"),
    ("HookFunction", SymbolKind::TypeAlias, "type HookFunction"),
    ("ConcurrentTestFn", SymbolKind::TypeAlias, "type ConcurrentTestFn"),
    ("EmptyFunction", SymbolKind::TypeAlias, "type EmptyFunction"),
    ("ArgsType", SymbolKind::TypeAlias, "type ArgsType<T>"),
    // Module-level functions
    ("fn", SymbolKind::Function, "function fn<T = any>(implementation?: (...args: any[]) => T): Mock<T>"),
    ("mock", SymbolKind::Function, "function mock(moduleName: string, factory?: () => unknown): typeof jest"),
    ("doMock", SymbolKind::Function, "function doMock(moduleName: string, factory?: () => unknown): typeof jest"),
    ("dontMock", SymbolKind::Function, "function dontMock(moduleName: string): typeof jest"),
    ("unmock", SymbolKind::Function, "function unmock(moduleName: string): typeof jest"),
    ("spyOn", SymbolKind::Function, "function spyOn<T>(object: T, method: keyof T): SpyInstance"),
    ("clearAllMocks", SymbolKind::Function, "function clearAllMocks(): typeof jest"),
    ("resetAllMocks", SymbolKind::Function, "function resetAllMocks(): typeof jest"),
    ("restoreAllMocks", SymbolKind::Function, "function restoreAllMocks(): typeof jest"),
    ("isMockFunction", SymbolKind::Function, "function isMockFunction(fn: any): boolean"),
    ("createMockFromModule", SymbolKind::Function, "function createMockFromModule<T = any>(moduleName: string): T"),
    ("requireActual", SymbolKind::Function, "function requireActual<T = any>(moduleName: string): T"),
    ("requireMock", SymbolKind::Function, "function requireMock<T = any>(moduleName: string): T"),
    ("setMock", SymbolKind::Function, "function setMock<T>(moduleName: string, moduleExports: T): void"),
    ("isolateModules", SymbolKind::Function, "function isolateModules(fn: () => void): void"),
    ("isolateModulesAsync", SymbolKind::Function, "function isolateModulesAsync(fn: () => Promise<void>): Promise<void>"),
    ("resetModules", SymbolKind::Function, "function resetModules(): typeof jest"),
    ("useFakeTimers", SymbolKind::Function, "function useFakeTimers(config?: any): typeof jest"),
    ("useRealTimers", SymbolKind::Function, "function useRealTimers(): typeof jest"),
    ("advanceTimersByTime", SymbolKind::Function, "function advanceTimersByTime(msToRun: number): void"),
    ("advanceTimersToNextTimer", SymbolKind::Function, "function advanceTimersToNextTimer(steps?: number): void"),
    ("runAllTicks", SymbolKind::Function, "function runAllTicks(): void"),
    ("runAllTimers", SymbolKind::Function, "function runAllTimers(): void"),
    ("runOnlyPendingTimers", SymbolKind::Function, "function runOnlyPendingTimers(): void"),
    ("clearAllTimers", SymbolKind::Function, "function clearAllTimers(): void"),
    ("getTimerCount", SymbolKind::Function, "function getTimerCount(): number"),
    ("setSystemTime", SymbolKind::Function, "function setSystemTime(now?: number | Date): void"),
    ("getRealSystemTime", SymbolKind::Function, "function getRealSystemTime(): number"),
    ("getSeed", SymbolKind::Function, "function getSeed(): number"),
    ("retryTimes", SymbolKind::Function, "function retryTimes(numRetries: number): typeof jest"),
    ("setTimeout", SymbolKind::Function, "function setTimeout(timeout: number): typeof jest"),
    ("now", SymbolKind::Function, "function now(): number"),
    ("disableAutomock", SymbolKind::Function, "function disableAutomock(): typeof jest"),
    ("enableAutomock", SymbolKind::Function, "function enableAutomock(): typeof jest"),
    ("genMockFromModule", SymbolKind::Function, "function genMockFromModule<T = any>(moduleName: string): T"),
    ("mocked", SymbolKind::Function, "function mocked<T>(item: T, deep?: boolean): Mocked<T>"),
    ("replaceProperty", SymbolKind::Function, "function replaceProperty<T, K extends keyof T>(object: T, key: K, value: T[K]): { restore(): void; replaceValue(value: T[K]): void }"),
];

/// `JSX.*` namespace — global namespace declared by `@types/react` AND
/// implicitly available in every TSX/JSX file that uses the `jsx` runtime.
/// Without `@types/react` on disk these are all unresolved.
const JSX_NAMESPACE_MEMBERS: &[(&str, SymbolKind, &str)] = &[
    ("Element", SymbolKind::Interface, "interface JSX.Element"),
    ("ElementClass", SymbolKind::Interface, "interface JSX.ElementClass"),
    ("ElementAttributesProperty", SymbolKind::Interface, "interface JSX.ElementAttributesProperty"),
    ("ElementChildrenAttribute", SymbolKind::Interface, "interface JSX.ElementChildrenAttribute"),
    ("LibraryManagedAttributes", SymbolKind::TypeAlias, "type JSX.LibraryManagedAttributes<C, P>"),
    ("IntrinsicAttributes", SymbolKind::Interface, "interface JSX.IntrinsicAttributes"),
    ("IntrinsicClassAttributes", SymbolKind::Interface, "interface JSX.IntrinsicClassAttributes<T>"),
    ("IntrinsicElements", SymbolKind::Interface, "interface JSX.IntrinsicElements"),
];

// =============================================================================
// Synthesis
// =============================================================================

fn sym_with_kind(name: &str, qualified_name: &str, kind: SymbolKind, signature: &str) -> ExtractedSymbol {
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

fn synthesize_globals_file() -> ParsedFile {
    let mut symbols = Vec::new();

    // Bare-name test globals — declared at module top level so the resolver's
    // by_name lookup finds them without any qualifier.
    for name in TEST_GLOBALS {
        symbols.push(sym_with_kind(
            name,
            name,
            SymbolKind::Function,
            &format!("function {name}(...args: any[]): any"),
        ));
    }

    // `jest` namespace — emit the namespace symbol plus each member under
    // `jest.<name>` so qualified lookups resolve.
    symbols.push(sym_with_kind("jest", "jest", SymbolKind::Namespace, "namespace jest"));
    for (name, kind, sig) in JEST_NAMESPACE_MEMBERS {
        symbols.push(sym_with_kind(name, &format!("jest.{name}"), *kind, sig));
    }

    // `vi` (vitest) — alias the same namespace shape; vitest's API mirrors
    // jest's at the module level. Members get their own qname under `vi.*`.
    symbols.push(sym_with_kind("vi", "vi", SymbolKind::Namespace, "namespace vi"));
    for (name, kind, sig) in JEST_NAMESPACE_MEMBERS {
        symbols.push(sym_with_kind(name, &format!("vi.{name}"), *kind, sig));
    }

    // `JSX` namespace — global in any TSX file. Members live under JSX.*.
    symbols.push(sym_with_kind("JSX", "JSX", SymbolKind::Namespace, "namespace JSX"));
    for (name, kind, sig) in JSX_NAMESPACE_MEMBERS {
        symbols.push(sym_with_kind(name, &format!("JSX.{name}"), *kind, sig));
    }

    let n_syms = symbols.len();
    ParsedFile {
        path: "ext:jest-synthetics:globals.d.ts".to_string(),
        language: "typescript".to_string(),
        content_hash: format!("jest-synthetics-{n_syms}"),
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
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
    }
}

// =============================================================================
// Synthetic dep root + Ecosystem impl
// =============================================================================

fn synthetic_dep_root() -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: "jest-synthetics".to_string(),
        version: String::new(),
        root: std::path::PathBuf::from("ext:jest-synthetics"),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

pub struct JestSyntheticsEcosystem;

impl Ecosystem for JestSyntheticsEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::LanguagePresent("typescript"),
            EcosystemActivation::LanguagePresent("tsx"),
            EcosystemActivation::LanguagePresent("javascript"),
            EcosystemActivation::LanguagePresent("jsx"),
            EcosystemActivation::LanguagePresent("vue"),
            EcosystemActivation::LanguagePresent("svelte"),
            EcosystemActivation::LanguagePresent("astro"),
            EcosystemActivation::LanguagePresent("angular"),
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
        Some(vec![synthesize_globals_file()])
    }
}

impl ExternalSourceLocator for JestSyntheticsEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }

    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        vec![synthetic_dep_root()]
    }

    fn parse_metadata_only(&self, _project_root: &Path) -> Option<Vec<ParsedFile>> {
        Some(vec![synthesize_globals_file()])
    }
}

#[cfg(test)]
#[path = "jest_synthetics_tests.rs"]
mod tests;
