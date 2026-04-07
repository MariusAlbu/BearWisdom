// =============================================================================
// typescript/primitives.rs — TypeScript/JavaScript primitive types
// =============================================================================

/// Primitive and built-in type names for TypeScript and JavaScript.
/// Includes keyword types, wrapper objects, common globals, and generic
/// type parameter names.
pub(crate) const PRIMITIVES: &[&str] = &[
    // Keyword types
    "string", "number", "boolean", "void", "null", "undefined", "any", "never",
    "object", "symbol", "bigint", "unknown",
    // Wrapper / global objects
    "String", "Number", "Boolean", "Object", "Array", "Function", "Symbol",
    "RegExp", "Date", "Error", "Promise", "Map", "Set",
    // Utility types (TypeScript built-in generics)
    "Record", "Partial", "Required", "Readonly", "Pick", "Omit",
    "Exclude", "Extract", "NonNullable", "ReturnType", "InstanceType",
    "ConstructorParameters", "Parameters", "ThisParameterType",
    "OmitThisParameter", "ThisType", "Awaited",
    // Error subtypes
    "TypeError", "RangeError", "SyntaxError", "ReferenceError", "EvalError",
    "URIError",
    // Typed arrays and buffers
    "ArrayBuffer", "SharedArrayBuffer", "DataView",
    "Uint8Array", "Uint16Array", "Uint32Array",
    "Int8Array", "Int16Array", "Int32Array",
    "Float32Array", "Float64Array", "BigInt64Array", "BigUint64Array",
    // Collections
    "WeakMap", "WeakSet", "WeakRef",
    // Browser / Node.js API types
    "Buffer", "Blob", "File", "FormData", "Headers", "Request", "Response",
    "URL", "URLSearchParams", "AbortController", "AbortSignal",
    "ReadableStream", "WritableStream", "TransformStream",
    "HTMLElement", "HTMLDivElement", "HTMLInputElement", "HTMLButtonElement",
    "HTMLImageElement", "HTMLFormElement", "HTMLAnchorElement",
    "HTMLCanvasElement", "HTMLVideoElement", "HTMLAudioElement",
    "HTMLSelectElement", "HTMLTextAreaElement", "HTMLSpanElement",
    "Element", "Node", "Document", "Event", "EventTarget",
    "MouseEvent", "KeyboardEvent", "FocusEvent", "InputEvent",
    "CustomEvent", "PointerEvent", "TouchEvent", "DragEvent",
    "IntersectionObserver", "MutationObserver", "ResizeObserver",
    "MessageEvent", "WebSocket", "Worker", "ServiceWorker",
    "Crypto", "CryptoKey", "TextEncoder", "TextDecoder",
    "MethodDecorator", "ClassDecorator", "PropertyDecorator", "ParameterDecorator",
    // Iterators / generators
    "Iterator", "AsyncIterator", "Generator", "AsyncGenerator",
    "IterableIterator", "AsyncIterableIterator",
    // Synthetic — emitted by extractor for primitive type annotations
    "_primitive",
    // TS keyword types (appear as type_ref in extractor output)
    "typeof", "keyof", "infer", "const", "readonly", "import",
    "asserts", "is", "out", "in", "extends", "implements",
    // React types
    "React.ComponentProps", "React.FC", "React.ReactNode", "React.ReactElement",
    "React.CSSProperties", "React.HTMLAttributes", "React.MouseEventHandler",
    "React.ChangeEventHandler", "React.FormEventHandler", "React.RefObject",
    "React.MutableRefObject", "React.Dispatch", "React.SetStateAction",
    "React.Context", "React.Provider", "React.Consumer",
    "JSX.Element", "JSX.IntrinsicElements",
    // Zod types (very common validation library)
    "z.infer", "z.input", "z.output", "z.ZodType", "z.ZodSchema",
    "z.ZodObject", "z.ZodArray", "z.ZodString", "z.ZodNumber", "z.ZodBoolean",
    "z.ZodEnum", "z.ZodUnion", "z.ZodOptional", "z.ZodNullable",
    // fp-ts / Effect types
    "E.Either", "O.Option", "TE.TaskEither", "T.Task", "IO.IO",
    "Either", "TaskEither", "Option", "Task", "IO",
    "pipe", "flow",
    // Generic type parameters
    "T", "U", "K", "V", "P", "R", "S", "E", "A", "B", "O",
];
