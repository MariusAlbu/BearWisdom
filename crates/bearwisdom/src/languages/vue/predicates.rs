// =============================================================================
// vue/builtins.rs — Vue builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
///
/// Delegates to TypeScript rules — Vue script blocks are TypeScript/JavaScript.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    crate::languages::typescript::predicates::kind_compatible(edge_kind, sym_kind)
}

/// Check whether a name is a Vue Composition API or Vue runtime builtin that
/// will never appear in the project symbol index.
///
/// This is called from the TypeScript resolver when the host file is `.vue` to
/// classify embedded `<script setup>` refs without explicit imports.
pub(crate) fn is_vue_builtin(name: &str) -> bool {
    let root = name.split('.').next().unwrap_or(name);

    // Delegate JS/TS runtime globals first.
    if crate::languages::typescript::predicates::is_js_runtime_global(root) {
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
            // ── Vue template instance properties ($-prefixed) ─────────────────
            // Injected by the Vue runtime on `this` in Options API and
            // accessible as `$x` in templates. Never in the project symbol index.
            | "$el"
            | "$refs"
            | "$attrs"
            | "$slots"
            | "$emit"
            | "$forceUpdate"
            | "$nextTick"
            | "$options"
            | "$parent"
            | "$props"
            | "$root"
            | "$data"
            | "$watch"
            | "$set"
            | "$delete"
            | "$on"
            | "$once"
            | "$off"
            // ── Vue I18n ──────────────────────────────────────────────────────
            | "$t"
            | "$tc"
            | "$te"
            | "$d"
            | "$n"
            | "$i18n"
            | "useI18n"
            // ── VueRouter instance properties ────────────────────────────────
            | "$router"
            | "$route"
            // ── Pinia instance ────────────────────────────────────────────────
            | "$store"
            // ── Inertia.js (@inertiajs/vue3) ─────────────────────────────────
            // Components and composables injected without explicit import in
            // script setup or accessed bare in templates.
            | "InertiaLink"
            | "Link"
            | "Head"
            | "usePage"
            | "useForm"
            | "router"
            | "route"
    )
}
