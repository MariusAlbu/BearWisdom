// =============================================================================
// vue/externals.rs — Vue external globals
// =============================================================================

use std::collections::HashSet;

/// Runtime globals always external for Vue components.
///
/// Includes browser baseline plus Vue 3 Composition API identifiers that
/// appear in `<script setup>` blocks without being project-defined.
pub(crate) const EXTERNALS: &[&str] = &[
    // ── Browser / universal baseline ─────────────────────────────────────────
    "console",
    "setTimeout",
    "setInterval",
    "clearTimeout",
    "clearInterval",
    "fetch",
    "URL",
    "URLSearchParams",
    "AbortController",
    "TextEncoder",
    "TextDecoder",
    "structuredClone",
    "atob",
    "btoa",
    "crypto",
    "performance",
    "document",
    "window",
    "navigator",
    "location",
    "history",
    "localStorage",
    "sessionStorage",
    "globalThis",
    // ── Vue 3 Composition API: reactivity ────────────────────────────────────
    "ref",
    "reactive",
    "readonly",
    "shallowRef",
    "shallowReactive",
    "shallowReadonly",
    "triggerRef",
    "customRef",
    "toRef",
    "toRefs",
    "toValue",
    "unref",
    "isRef",
    "isReactive",
    "isReadonly",
    "isProxy",
    "isShallow",
    "toRaw",
    "markRaw",
    // ── Computed / watch ──────────────────────────────────────────────────────
    "computed",
    "watch",
    "watchEffect",
    "watchPostEffect",
    "watchSyncEffect",
    // ── Effect scope ──────────────────────────────────────────────────────────
    "effectScope",
    "getCurrentScope",
    "onScopeDispose",
    // ── Lifecycle hooks ───────────────────────────────────────────────────────
    "onMounted",
    "onUpdated",
    "onUnmounted",
    "onBeforeMount",
    "onBeforeUpdate",
    "onBeforeUnmount",
    "onErrorCaptured",
    "onRenderTracked",
    "onRenderTriggered",
    "onActivated",
    "onDeactivated",
    "onServerPrefetch",
    // ── Dependency injection ──────────────────────────────────────────────────
    "provide",
    "inject",
    // ── Component utilities ───────────────────────────────────────────────────
    "defineComponent",
    "defineProps",
    "defineEmits",
    "defineExpose",
    "defineSlots",
    "defineModel",
    "defineOptions",
    "withDefaults",
    "useSlots",
    "useAttrs",
    "useModel",
    "useTemplateRef",
    "useId",
    "h",
    "createApp",
    "nextTick",
    "defineAsyncComponent",
    "defineCustomElement",
    "resolveComponent",
    "resolveDirective",
    "withDirectives",
    "withModifiers",
];

/// Dependency-gated framework globals for Vue.
pub(crate) fn framework_globals(deps: &HashSet<String>) -> Vec<&'static str> {
    let mut globals = Vec::new();

    // Inherit JS/TS framework globals (test runners, i18n, etc.)
    globals.extend(crate::languages::typescript::externals::framework_globals(deps));

    // Vue Router
    if deps.contains("vue-router") {
        globals.extend(VUE_ROUTER_GLOBALS);
    }

    // Pinia state management
    if deps.contains("pinia") {
        globals.extend(PINIA_GLOBALS);
    }

    // Vuex (Vue 2 / legacy)
    if deps.contains("vuex") {
        globals.extend(VUEX_GLOBALS);
    }

    // Vue Test Utils
    for dep in ["@vue/test-utils", "vue-test-utils"] {
        if deps.contains(dep) {
            globals.extend(VUE_TEST_UTILS_GLOBALS);
            break;
        }
    }

    // Nuxt
    for dep in ["nuxt", "nuxt3", "@nuxt/kit", "@nuxtjs/composition-api"] {
        if deps.contains(dep) {
            globals.extend(NUXT_GLOBALS);
            break;
        }
    }

    globals
}

const VUE_ROUTER_GLOBALS: &[&str] = &[
    "useRoute",
    "useRouter",
    "useLink",
    "RouterView",
    "RouterLink",
    "createRouter",
    "createWebHistory",
    "createWebHashHistory",
    "createMemoryHistory",
];

const PINIA_GLOBALS: &[&str] = &[
    "defineStore",
    "storeToRefs",
    "acceptHMRUpdate",
    "createPinia",
    "setActivePinia",
    "getActivePinia",
];

const VUEX_GLOBALS: &[&str] = &[
    "createStore",
    "useStore",
    "mapState",
    "mapGetters",
    "mapActions",
    "mapMutations",
    "createNamespacedHelpers",
];

const VUE_TEST_UTILS_GLOBALS: &[&str] = &[
    "mount",
    "shallowMount",
    "flushPromises",
    "DOMWrapper",
    "VueWrapper",
    "config",
    "enableAutoUnmount",
];

const NUXT_GLOBALS: &[&str] = &[
    "useNuxtApp",
    "useRuntimeConfig",
    "useRoute",
    "useRouter",
    "useHead",
    "useSeoMeta",
    "useAsyncData",
    "useFetch",
    "useLazyAsyncData",
    "useLazyFetch",
    "useError",
    "useNuxtData",
    "clearError",
    "navigateTo",
    "abortNavigation",
    "defineNuxtPlugin",
    "defineNuxtRouteMiddleware",
    "defineEventHandler",
    "readBody",
    "getQuery",
    "createError",
    "useState",
    "clearNuxtData",
    "refreshNuxtData",
    "setResponseStatus",
    "setPageLayout",
    "preloadComponents",
    "prefetchComponents",
    "isPrerendered",
];
