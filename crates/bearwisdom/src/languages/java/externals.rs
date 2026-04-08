use std::collections::HashSet;

/// Runtime globals always external for Java.
pub(crate) const EXTERNALS: &[&str] = &[
    // SLF4J logging (ubiquitous)
    "Logger", "LoggerFactory",
];

/// Dependency-gated framework globals for Java.
pub(crate) fn framework_globals(deps: &HashSet<String>) -> Vec<&'static str> {
    let mut globals = Vec::new();

    // Spring framework
    for dep in [
        "org.springframework.boot:spring-boot-starter-test",
        "org.springframework.boot",
        "spring-boot-starter-test",
        "org.springframework",
    ] {
        if deps.contains(dep) {
            globals.extend(SPRING_CORE_GLOBALS);
            globals.extend(SPRING_TEST_GLOBALS);
            break;
        }
    }

    // JUnit / Mockito
    for dep in ["junit", "org.junit.jupiter", "org.junit"] {
        if deps.contains(dep) {
            globals.extend(JUNIT_GLOBALS);
            break;
        }
    }

    // JavaFX
    for dep in ["javafx", "org.openjfx", "javafx-controls", "javafx-base"] {
        if deps.contains(dep) {
            globals.extend(JAVAFX_GLOBALS);
            break;
        }
    }

    // ASM (bytecode manipulation)
    for dep in ["org.ow2.asm", "asm", "asm-tree", "asm-commons"] {
        if deps.contains(dep) {
            globals.extend(ASM_GLOBALS);
            break;
        }
    }

    globals
}

const SPRING_CORE_GLOBALS: &[&str] = &[
    "RestController", "Controller", "Service", "Component", "Repository",
    "Configuration", "Bean", "Autowired", "Value", "Qualifier", "Primary",
    "Transactional", "Scheduled", "EventListener", "Async",
    "RequestMapping", "GetMapping", "PostMapping", "PutMapping", "DeleteMapping",
    "PatchMapping",
    "PathVariable", "RequestBody", "RequestParam", "RequestHeader",
    "ResponseEntity", "HttpStatus", "MediaType",
    "PageRequest", "Pageable", "Page", "Sort", "Specification",
];

const SPRING_TEST_GLOBALS: &[&str] = &[
    "status", "content", "jsonPath", "xpath", "header", "cookie", "flash",
    "model", "view", "forwardedUrl", "redirectedUrl", "redirectedUrlPattern",
    "isOk", "isCreated", "isAccepted", "isNoContent",
    "isBadRequest", "isUnauthorized", "isForbidden", "isNotFound",
    "isConflict", "isInternalServerError",
    "contentType", "contentTypeCompatibleWith",
    "get", "post", "put", "patch", "delete", "options", "head",
    "accept", "param", "params", "multipart",
    "perform", "andExpect", "andReturn", "andDo",
    "MockBean", "SpyBean", "WebMvcTest", "SpringBootTest",
    "DataJpaTest", "AutoConfigureMockMvc",
    "assertThat", "isEqualTo", "isNotNull", "isNull", "isTrue", "isFalse",
    "hasSize", "contains", "containsExactly", "isEmpty", "isNotEmpty",
    "isInstanceOf", "extracting", "satisfies",
];

const JUNIT_GLOBALS: &[&str] = &[
    "assertEquals", "assertThat", "assertTrue", "assertFalse",
    "assertNull", "assertNotNull", "verify", "when", "given", "mock",
];

const JAVAFX_GLOBALS: &[&str] = &[
    "Label", "Button", "TextField", "TextArea", "CheckBox", "ComboBox",
    "ListView", "TableView", "TreeView", "TreeItem",
    "Node", "Parent", "Region", "Pane", "StackPane", "BorderPane",
    "HBox", "VBox", "GridPane", "FlowPane", "AnchorPane", "ScrollPane",
    "Scene", "Stage", "Window",
    "Tab", "TabPane", "SplitPane", "TitledPane", "Accordion",
    "Menu", "MenuItem", "MenuBar", "ContextMenu", "ToolBar",
    "Alert", "Dialog", "FileChooser", "DirectoryChooser",
    "ImageView", "Image", "Canvas", "WebView",
    "Insets", "Color", "Font", "Cursor",
    "KeyEvent", "MouseEvent", "ActionEvent",
    "FXMLLoader", "FXML",
    "Platform", "Application", "Task", "Service",
    "ObservableValue", "ObservableList", "ObservableMap", "ObservableSet",
    "SimpleStringProperty", "SimpleIntegerProperty", "SimpleBooleanProperty",
    "SimpleObjectProperty", "SimpleDoubleProperty",
    "BooleanProperty", "StringProperty", "IntegerProperty", "DoubleProperty",
    "ObjectProperty", "ListProperty", "MapProperty",
    "ReadOnlyBooleanProperty", "ReadOnlyStringProperty", "ReadOnlyObjectProperty",
    "ObservableObject", "ObservableBoolean",
    "Binding", "Bindings",
    "FadeTransition", "TranslateTransition", "ScaleTransition",
    "ChangeListener", "InvalidationListener",
    "Tooltip", "Separator", "ProgressBar", "ProgressIndicator", "Slider",
    "Spinner", "ToggleButton", "RadioButton", "ToggleGroup", "Hyperlink",
    "Pagination", "DatePicker", "ColorPicker",
    "LinearGradient", "RadialGradient", "Stop",
];

const ASM_GLOBALS: &[&str] = &[
    "AbstractInsnNode", "InsnNode", "InsnList",
    "AnnotationVisitor", "ClassVisitor", "ClassWriter", "ClassReader",
    "MethodVisitor", "FieldVisitor",
    "LabelNode", "TypePath", "Type", "Handle", "Opcodes",
    "MethodInsnNode", "FieldInsnNode", "VarInsnNode", "JumpInsnNode",
    "LdcInsnNode", "TypeInsnNode", "IntInsnNode", "IincInsnNode",
    "TableSwitchInsnNode", "LookupSwitchInsnNode", "MultiANewArrayInsnNode",
    "FrameNode", "LineNumberNode", "LocalVariableNode",
];
