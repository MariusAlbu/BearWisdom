// =============================================================================
// vue/builtins.rs — Vue builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
///
/// Delegates to TypeScript rules — Vue script blocks are TypeScript/JavaScript.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    crate::languages::typescript::builtins::kind_compatible(edge_kind, sym_kind)
}

/// Check whether a name is a Vue Composition API or Vue runtime builtin that
/// will never appear in the project symbol index.
pub(super) fn is_vue_builtin(name: &str) -> bool {
    let root = name.split('.').next().unwrap_or(name);

    // Delegate JS/TS runtime globals first.
    if crate::languages::typescript::builtins::is_js_runtime_global(root) {
        return true;
    }

    matches!(
        root,
        // ── Reactivity: ref / reactive family ────────────────────────────────
        "ref"
            | "reactive"
            | "readonly"
            | "shallowRef"
            | "shallowReactive"
            | "shallowReadonly"
            | "triggerRef"
            | "customRef"
            | "toRef"
            | "toRefs"
            | "toValue"
            | "unref"
            | "isRef"
            | "isReactive"
            | "isReadonly"
            | "isProxy"
            | "isShallow"
            | "toRaw"
            | "markRaw"
            // ── Computed / watch ─────────────────────────────────────────────
            | "computed"
            | "watch"
            | "watchEffect"
            | "watchPostEffect"
            | "watchSyncEffect"
            // ── Effect scope ─────────────────────────────────────────────────
            | "effectScope"
            | "getCurrentScope"
            | "onScopeDispose"
            // ── Lifecycle hooks ──────────────────────────────────────────────
            | "onMounted"
            | "onUpdated"
            | "onUnmounted"
            | "onBeforeMount"
            | "onBeforeUpdate"
            | "onBeforeUnmount"
            | "onErrorCaptured"
            | "onRenderTracked"
            | "onRenderTriggered"
            | "onActivated"
            | "onDeactivated"
            | "onServerPrefetch"
            // ── Dependency injection ─────────────────────────────────────────
            | "provide"
            | "inject"
            // ── Component utilities ──────────────────────────────────────────
            | "defineComponent"
            | "defineProps"
            | "defineEmits"
            | "defineExpose"
            | "defineSlots"
            | "defineModel"
            | "defineOptions"
            | "withDefaults"
            | "useSlots"
            | "useAttrs"
            | "useModel"
            | "useTemplateRef"
            | "useId"
            | "h"
            | "createApp"
            | "nextTick"
            | "defineAsyncComponent"
            | "defineCustomElement"
            | "resolveComponent"
            | "resolveDirective"
            | "withDirectives"
            | "withModifiers"
            | "vModelText"
            | "vModelCheckbox"
            | "vModelRadio"
            | "vModelSelect"
            | "vModelDynamic"
            | "vShow"
            // ── VueRouter ─────────────────────────────────────────────────────
            | "useRoute"
            | "useRouter"
            | "useLink"
            // ── Pinia ────────────────────────────────────────────────────────
            | "defineStore"
            | "storeToRefs"
            | "useStore"
            // ── Vue Test Utils ───────────────────────────────────────────────
            | "mount"
            | "shallowMount"
            | "flushPromises"
    )
}
