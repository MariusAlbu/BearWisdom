// =============================================================================
// nuget/dotscope_worker.rs — the dedicated dotscope thread and its per-run
// assembly cache.
//
// Owns all serialized access to `dotscope`: the mailbox, the worker thread,
// the parse-once-per-run `CilObject` cache, and the client entry points
// (`crack_one_dll_type`, `flush_assembly_cache`). Type extraction itself
// lives in `dll_metadata` — the worker only decides when a DLL is parsed.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use once_cell::sync::Lazy;

use super::assembly_cache::{AssemblyCache, DEFAULT_BUDGET};
use super::dll_metadata::extract_type_from_assembly;

/// A request to crack one type out of a DLL, sent to the dedicated dotscope
/// thread. `reply` carries the resulting `ParsedFile` (or `None`) back.
struct CrackRequest {
    dll_path: PathBuf,
    qualified_type: String,
    lang_id: String,
    virtual_path: String,
    reply: std::sync::mpsc::Sender<Option<crate::types::ParsedFile>>,
}

/// Worker mailbox: crack one type, or drop the per-run assembly cache.
enum DotscopeMsg {
    Crack(CrackRequest),
    Flush,
}

/// Drops the dotscope worker's assembly cache. Runs at index-run boundaries
/// via `Ecosystem::reset_demand_caches`. Channel FIFO guarantees the flush
/// lands before any crack queued after it, so no ack is needed.
pub(crate) fn flush_assembly_cache() {
    if let Ok(tx) = DOTSCOPE_TX.lock() {
        let _ = tx.send(DotscopeMsg::Flush);
    }
}

/// All `dotscope` work runs on ONE dedicated OS thread, never on the resolve
/// pool's rayon workers.
///
/// Two reasons this is mandatory: (1) `dotscope` is not safe to use from
/// multiple threads at once (parallel parse OR read deadlocks); (2) `dotscope`
/// itself uses rayon — calling it from a resolve-pool worker nests its parallel
/// work on the resolve pool, which deadlocks when the other workers are blocked
/// waiting on this DLL. Running on a plain (non-rayon) thread routes dotscope's
/// internal rayon to the idle global pool instead, and the single thread
/// serializes access. The per-DLL `CilObject` cache lives on this thread, so
/// it needs no lock; a DLL parses at most once per index run (`Flush` marks
/// the run boundary).
static DOTSCOPE_TX: Lazy<Mutex<std::sync::mpsc::Sender<DotscopeMsg>>> = Lazy::new(|| {
    let (tx, rx) = std::sync::mpsc::channel::<DotscopeMsg>();
    std::thread::Builder::new()
        .name("bw-dotscope".into())
        .spawn(move || {
            let mut cache: AssemblyCache<Arc<dotscope::prelude::CilObject>> =
                AssemblyCache::new(DEFAULT_BUDGET);
            while let Ok(msg) = rx.recv() {
                let req = match msg {
                    DotscopeMsg::Flush => {
                        cache.clear();
                        continue;
                    }
                    DotscopeMsg::Crack(req) => req,
                };
                let assembly = cache
                    .get_or_load(&req.dll_path, |p| load_assembly(p).map(|a| (Arc::new(a), 1)));
                let result = assembly.and_then(|asm| {
                    extract_type_from_assembly(
                        &asm,
                        &req.qualified_type,
                        &req.lang_id,
                        &req.virtual_path,
                        &req.dll_path,
                    )
                });
                let _ = req.reply.send(result);
            }
        })
        .expect("failed to spawn dotscope worker thread");
    Mutex::new(tx)
});

/// Parse one DLL into a `CilObject`. Only ever called on the dotscope thread.
fn load_assembly(dll_path: &Path) -> Option<dotscope::prelude::CilObject> {
    use dotscope::metadata::cilassemblyview::CilAssemblyView;
    use dotscope::metadata::validation::ValidationConfig;
    use dotscope::prelude::CilObject;

    let mut config = ValidationConfig::disabled();
    config.lenient = true;
    let view = CilAssemblyView::from_path_with_validation(dll_path, config.clone()).ok()?;
    CilObject::from_view_with_validation(view, config).ok()
}

/// Crack a single .NET type from a DLL on demand. `virtual_path` must be in
/// the `ext:dotnet-type:<dll_path>!!<assembly_name>!!<qualified_type>` form
/// produced by `list_dll_type_names`. Returns `None` on any decoding or I/O
/// error so the caller can skip gracefully.
pub(crate) fn crack_one_dll_type(
    virtual_path: &str,
    lang_id: &str,
) -> Option<crate::types::ParsedFile> {
    // Decode "ext:dotnet-type:<dll_path>!!<assembly_name>!!<qualified_type>"
    let payload = virtual_path.strip_prefix("ext:dotnet-type:")?;
    let mut parts = payload.splitn(3, "!!");
    let dll_str = parts.next()?;
    let _assembly_name = parts.next()?;
    let qualified_type = parts.next()?;

    // Hand the crack to the dedicated dotscope thread and block for the reply —
    // dotscope must never run on a resolve-pool worker (see `DOTSCOPE_TX`).
    let (reply, reply_rx) = std::sync::mpsc::channel();
    let req = CrackRequest {
        dll_path: PathBuf::from(dll_str),
        qualified_type: qualified_type.to_string(),
        lang_id: lang_id.to_string(),
        virtual_path: virtual_path.to_string(),
        reply,
    };
    DOTSCOPE_TX.lock().ok()?.send(DotscopeMsg::Crack(req)).ok()?;
    reply_rx.recv().ok()?
}
