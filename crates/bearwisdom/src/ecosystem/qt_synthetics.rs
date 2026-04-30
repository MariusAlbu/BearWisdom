// =============================================================================
// ecosystem/qt_synthetics.rs — Qt framework synthetic stubs (C++)
//
// Qt's public API (QObject / QString / QList / Q_OBJECT macro / signal/slot
// pseudo-functions / qDebug / QCOMPARE / QVERIFY) is the dominant single
// source of unresolved refs in C++ Qt projects. cpp-keepassxc alone has
// ~37K unresolved refs — every QCOMPARE in a test file, every Q_OBJECT in
// a class declaration, every `connect(this, SIGNAL(...), …)`.
//
// The architecturally correct fix is to discover the Qt installation from
// `qmake` / CMakeLists.txt and parse the actual Qt headers — that's a
// substantial walker effort and depends on a specific Qt version being
// installed. As a complement, this synthetic ecosystem emits the public
// surface as plain C++ symbols so the resolver's `by_name` step routes
// every QObject/Q_OBJECT/QCOMPARE bare-name reference to a known target.
//
// Mirrors `spring_stubs` / `phoenix_stubs` / `jest_synthetics` — synthetic
// symbols sit harmless when the project doesn't use Qt. Activates on any
// project with C or C++ files present.
//
// Inventory is biased toward the names that produced 100+ unresolved hits
// in the corpus audit. The list is dense but each entry maps to a real
// public Qt symbol — we are not inventing API.
// =============================================================================

use std::path::Path;

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("qt-synthetics");
const LEGACY_ECOSYSTEM_TAG: &str = "qt-synthetics";
const LANGUAGES: &[&str] = &["c", "cpp"];

// Qt test framework macros (QtTest module).
const QT_TEST_MACROS: &[&str] = &[
    "QCOMPARE", "QVERIFY", "QVERIFY2", "QFAIL", "QFETCH", "QFETCH_GLOBAL",
    "QSKIP", "QEXPECT_FAIL", "QWARN", "QBENCHMARK", "QBENCHMARK_ONCE",
    "QTRY_COMPARE", "QTRY_VERIFY", "QTRY_VERIFY2", "QTRY_VERIFY_WITH_TIMEOUT",
    "QTRY_COMPARE_WITH_TIMEOUT", "QTRY_IMPL",
    "QTEST_MAIN", "QTEST_APPLESS_MAIN", "QTEST_GUILESS_MAIN",
    "QTEST_NAMESPACE", "QTEST_REGISTER_OBJECT",
    "QTEST_FRAMEWORK_LIB_NAME",
    "QTRY_LOOP_IMPL",
];

// Qt object-system macros (moc-related).
const QT_OBJECT_MACROS: &[&str] = &[
    "Q_OBJECT", "Q_GADGET", "Q_NAMESPACE", "Q_NAMESPACE_EXPORT",
    "Q_PROPERTY", "Q_INVOKABLE", "Q_SIGNALS", "Q_SLOTS",
    "Q_EMIT", "Q_ENUM", "Q_ENUMS", "Q_FLAG", "Q_FLAGS",
    "Q_DECLARE_METATYPE", "Q_DECLARE_FLAGS", "Q_DECLARE_OPERATORS_FOR_FLAGS",
    "Q_DECLARE_INTERFACE", "Q_DECLARE_OPAQUE_POINTER",
    "Q_DECLARE_PRIVATE", "Q_DECLARE_PUBLIC", "Q_DECLARE_TR_FUNCTIONS",
    "Q_DISABLE_COPY", "Q_DISABLE_COPY_MOVE", "Q_DISABLE_MOVE",
    "Q_INTERFACES", "Q_PLUGIN_METADATA", "Q_REVISION",
    "Q_CLASSINFO", "Q_DECL_OVERRIDE", "Q_DECL_FINAL", "Q_DECL_NULLPTR",
    "Q_DECL_DEPRECATED", "Q_DECL_NOEXCEPT", "Q_DECL_CONSTEXPR",
    "Q_DECL_RELAXED_CONSTEXPR", "Q_DECL_UNUSED", "Q_DECL_NOTHROW",
    "Q_DECL_HIDDEN", "Q_DECL_EXPORT", "Q_DECL_IMPORT",
    "Q_GLOBAL_STATIC", "Q_GLOBAL_STATIC_WITH_ARGS",
    "Q_LOGGING_CATEGORY", "Q_DECLARE_LOGGING_CATEGORY",
    "Q_OBJECT_FAKE", "Q_MOC_INCLUDE", "Q_INTERFACE_MAJOR_VERSION",
    "QObject_traits",
];

// Qt utility / control-flow macros.
const QT_UTILITY_MACROS: &[&str] = &[
    "Q_ASSERT", "Q_ASSERT_X", "Q_CHECK_PTR", "Q_UNUSED", "Q_UNREACHABLE",
    "Q_FOREACH", "Q_FOREVER", "Q_LIKELY", "Q_UNLIKELY",
    "Q_RETURN_ARG", "Q_ARG", "Q_NULLPTR",
    "Q_FUNC_INFO", "Q_FALLTHROUGH",
    "QT_REQUIRE_VERSION", "QT_VERSION_CHECK", "QT_TRANSLATE_NOOP",
    "QT_TR_NOOP", "QT_TRID_NOOP",
    "QT_BEGIN_NAMESPACE", "QT_END_NAMESPACE",
    "QT_BEGIN_INCLUDE_NAMESPACE", "QT_END_INCLUDE_NAMESPACE",
    "QT_FORWARD_DECLARE_CLASS", "QT_FORWARD_DECLARE_STRUCT",
    "QT_USE_NAMESPACE", "QT_PREPEND_NAMESPACE",
    "QT_DEPRECATED", "QT_DEPRECATED_X",
    "QT_VERSION", "QT_VERSION_STR",
    "QT_TRANSLATE_NOOP3", "QT_TRANSLATE_NOOP3_UTF8",
    "tr", "trUtf8",
];

// Qt signal/slot pseudo-functions used in `connect()` calls.
const QT_SIGNAL_SLOT_MARKERS: &[&str] = &[
    "SIGNAL", "SLOT", "METHOD",
];

// Qt logging functions.
const QT_LOGGING: &[&str] = &[
    "qDebug", "qInfo", "qWarning", "qCritical", "qFatal",
    "qInstallMessageHandler", "qSetMessagePattern",
    "qErrnoWarning", "qPrintable", "qUtf8Printable",
    "qFormatLogMessage", "qDebugBuiltIn",
    // qCDebug variants
    "qCDebug", "qCInfo", "qCWarning", "qCCritical", "qCFatal",
];

// Qt math / utility free functions.
const QT_MATH: &[&str] = &[
    "qAbs", "qBound", "qFuzzyCompare", "qFuzzyIsNull",
    "qMin", "qMax", "qPow", "qSqrt", "qExp", "qLn",
    "qSin", "qCos", "qTan", "qAtan", "qAtan2",
    "qFloor", "qCeil", "qRound", "qRound64",
    "qrand", "qsrand", "qHash", "qHashBits", "qHashRange",
    "qMakeFinite", "qIsFinite", "qIsInf", "qIsNaN",
    "qsnprintf", "qvsnprintf",
    "qstrcmp", "qstricmp", "qstrlen", "qstrncmp", "qstrnicmp",
    "qstrcpy", "qstrncpy", "qstrdup",
    "qFromBigEndian", "qToBigEndian", "qFromLittleEndian", "qToLittleEndian",
    "qbswap", "qbswap_helper",
    "qPopulationCount", "qCountLeadingZeroBits", "qCountTrailingZeroBits",
];

// Qt integer typedefs.
const QT_TYPEDEFS: &[&str] = &[
    "qint8", "qint16", "qint32", "qint64", "qintptr",
    "quint8", "quint16", "quint32", "quint64", "quintptr",
    "qreal", "qsizetype", "qptrdiff",
    "QRgb", "QRgba64",
    "QChar", "QLatin1Char", "QLatin1String",
    "QStringView", "QByteArrayView",
    "QFlag", "QFlags",
];

// Common Qt classes — names that show up as type references.
const QT_CLASSES: &[&str] = &[
    // Core
    "QObject", "QString", "QByteArray", "QVariant",
    "QList", "QVector", "QMap", "QHash", "QSet", "QPair", "QQueue", "QStack",
    "QLinkedList", "QStringList", "QVariantList", "QVariantMap", "QVariantHash",
    "QSharedPointer", "QScopedPointer", "QPointer", "QWeakPointer",
    "QSharedDataPointer", "QExplicitlySharedDataPointer", "QSharedData",
    "QFlag", "QFlags",
    "QChar", "QLatin1Char", "QLatin1String", "QLatin1StringView",
    "QStringView", "QByteArrayView", "QStringRef", "QStringMatcher",
    "QRegExp", "QRegularExpression", "QRegularExpressionMatch",
    "QRegularExpressionMatchIterator",
    "QDate", "QTime", "QDateTime", "QTimeZone", "QCalendar",
    "QUrl", "QUrlQuery", "QMimeType", "QMimeData", "QMimeDatabase",
    "QFile", "QFileInfo", "QDir", "QFileSystemModel", "QFileSystemWatcher",
    "QSaveFile", "QTemporaryFile", "QTemporaryDir",
    "QIODevice", "QBuffer", "QDataStream", "QTextStream", "QDebug",
    "QProcess", "QThread", "QThreadPool", "QRunnable", "QFuture", "QFutureWatcher",
    "QMutex", "QMutexLocker", "QSemaphore", "QWaitCondition", "QReadWriteLock",
    "QReadLocker", "QWriteLocker", "QAtomicInt", "QAtomicInteger", "QAtomicPointer",
    "QTimer", "QTimerEvent", "QBasicTimer",
    "QEvent", "QChildEvent", "QTimerEvent", "QDynamicPropertyChangeEvent",
    "QCoreApplication", "QApplication", "QGuiApplication",
    "QSignalMapper", "QSignalBlocker",
    "QMetaObject", "QMetaProperty", "QMetaMethod", "QMetaEnum", "QMetaClassInfo",
    "QMetaType",
    "QSettings", "QStandardPaths",
    "QLocale", "QTranslator", "QLibraryInfo",
    "QCommandLineParser", "QCommandLineOption",
    "QLoggingCategory",
    "QPluginLoader", "QLibrary",
    "QCryptographicHash", "QMessageAuthenticationCode",
    "QUuid", "QSysInfo",
    "QException", "QUnhandledException",
    // GUI
    "QWidget", "QMainWindow", "QDialog", "QMessageBox", "QFileDialog",
    "QInputDialog", "QFontDialog", "QColorDialog",
    "QLabel", "QPushButton", "QToolButton", "QCheckBox", "QRadioButton",
    "QLineEdit", "QTextEdit", "QPlainTextEdit", "QSpinBox", "QDoubleSpinBox",
    "QComboBox", "QSlider", "QDial", "QProgressBar", "QScrollBar",
    "QTabBar", "QTabWidget", "QToolBar", "QMenuBar", "QMenu", "QStatusBar",
    "QListWidget", "QListView", "QListWidgetItem",
    "QTreeWidget", "QTreeView", "QTreeWidgetItem",
    "QTableWidget", "QTableView", "QTableWidgetItem",
    "QStandardItem", "QStandardItemModel", "QSortFilterProxyModel",
    "QAbstractItemModel", "QAbstractListModel", "QAbstractTableModel",
    "QAction", "QActionGroup", "QShortcut", "QKeySequence",
    "QSplitter", "QSplitterHandle", "QStackedWidget", "QStackedLayout",
    "QGroupBox", "QFrame", "QScrollArea", "QAbstractScrollArea",
    "QHBoxLayout", "QVBoxLayout", "QGridLayout", "QFormLayout", "QBoxLayout",
    "QLayout", "QLayoutItem", "QSpacerItem", "QSizePolicy",
    "QPainter", "QPen", "QBrush", "QColor", "QFont", "QFontMetrics",
    "QPalette", "QIcon", "QPixmap", "QImage", "QImageReader", "QImageWriter",
    "QPicture", "QBitmap", "QCursor", "QClipboard",
    "QScreen", "QWindow", "QPaintDevice", "QPaintEngine",
    "QPainterPath", "QPolygon", "QPolygonF", "QRegion",
    "QRect", "QRectF", "QSize", "QSizeF", "QPoint", "QPointF",
    "QLine", "QLineF", "QMargins", "QMarginsF", "QTransform", "QMatrix4x4",
    "QGraphicsView", "QGraphicsScene", "QGraphicsItem", "QGraphicsObject",
    "QGraphicsRectItem", "QGraphicsEllipseItem", "QGraphicsTextItem",
    "QGraphicsLineItem", "QGraphicsPolygonItem", "QGraphicsPathItem",
    "QGraphicsPixmapItem", "QGraphicsItemGroup",
    "QGraphicsProxyWidget", "QGraphicsLayout", "QGraphicsLinearLayout",
    "QGraphicsAnchorLayout", "QGraphicsGridLayout",
    "QStyle", "QStyleFactory", "QStyleOption",
    "QValidator", "QIntValidator", "QDoubleValidator", "QRegExpValidator",
    "QRegularExpressionValidator",
    "QCompleter", "QStringListModel",
    "QHeaderView", "QItemSelection", "QItemSelectionModel",
    "QItemDelegate", "QStyledItemDelegate", "QAbstractItemDelegate",
    "QModelIndex", "QPersistentModelIndex",
    "QDrag", "QDropEvent", "QDragEnterEvent", "QDragMoveEvent", "QDragLeaveEvent",
    "QMouseEvent", "QKeyEvent", "QWheelEvent", "QFocusEvent", "QPaintEvent",
    "QResizeEvent", "QShowEvent", "QHideEvent", "QCloseEvent", "QMoveEvent",
    // Network
    "QNetworkAccessManager", "QNetworkRequest", "QNetworkReply",
    "QNetworkProxy", "QNetworkInterface", "QNetworkAddressEntry",
    "QHostAddress", "QHostInfo", "QTcpServer", "QTcpSocket",
    "QUdpSocket", "QSslSocket", "QSslConfiguration", "QSslCertificate",
    "QSslKey", "QSslError", "QSslCipher", "QSslSocket", "QAbstractSocket",
    "QLocalServer", "QLocalSocket",
    // SQL
    "QSqlDatabase", "QSqlQuery", "QSqlRecord", "QSqlField", "QSqlError",
    "QSqlTableModel", "QSqlRelationalTableModel", "QSqlQueryModel",
    "QSqlDriver", "QSqlIndex",
    // XML / JSON
    "QJsonDocument", "QJsonObject", "QJsonArray", "QJsonValue", "QJsonParseError",
    "QXmlStreamReader", "QXmlStreamWriter", "QXmlStreamAttributes",
    "QDomDocument", "QDomElement", "QDomNode", "QDomNodeList",
    // Concurrent
    "QtConcurrent", "QFutureSynchronizer",
    // DBus
    "QDBusConnection", "QDBusInterface", "QDBusMessage", "QDBusReply",
    "QDBusObjectPath", "QDBusSignature", "QDBusVariant", "QDBusError",
    "QDBusArgument", "QDBusAbstractInterface",
    // Test
    "QTest", "QSignalSpy", "QAbstractItemModelTester",
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

fn synthesize_file() -> ParsedFile {
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for name in QT_TEST_MACROS
        .iter()
        .chain(QT_OBJECT_MACROS)
        .chain(QT_UTILITY_MACROS)
        .chain(QT_SIGNAL_SLOT_MARKERS)
    {
        if seen.insert(*name) {
            symbols.push(sym(
                name,
                name,
                SymbolKind::Function,
                &format!("#define {name}(...) /* Qt macro */"),
            ));
        }
    }
    for name in QT_LOGGING.iter().chain(QT_MATH) {
        if seen.insert(*name) {
            symbols.push(sym(
                name,
                name,
                SymbolKind::Function,
                &format!("Qt free function {name}(...)"),
            ));
        }
    }
    for name in QT_TYPEDEFS {
        if seen.insert(*name) {
            symbols.push(sym(
                name,
                name,
                SymbolKind::TypeAlias,
                &format!("typedef {name}"),
            ));
        }
    }
    for name in QT_CLASSES {
        if seen.insert(*name) {
            symbols.push(sym(
                name,
                name,
                SymbolKind::Class,
                &format!("class {name}"),
            ));
        }
    }

    let n_syms = symbols.len();
    ParsedFile {
        path: "ext:qt-synthetics:Qt.h".to_string(),
        language: "cpp".to_string(),
        content_hash: format!("qt-synthetics-{n_syms}"),
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
        module_path: "qt-synthetics".to_string(),
        version: String::new(),
        root: std::path::PathBuf::from("ext:qt-synthetics"),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

pub struct QtSyntheticsEcosystem;

impl Ecosystem for QtSyntheticsEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::LanguagePresent("c"),
            EcosystemActivation::LanguagePresent("cpp"),
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

impl ExternalSourceLocator for QtSyntheticsEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }

    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        vec![synthetic_dep_root()]
    }

    fn parse_metadata_only(&self, _project_root: &Path) -> Option<Vec<ParsedFile>> {
        Some(vec![synthesize_file()])
    }
}

#[cfg(test)]
#[path = "qt_synthetics_tests.rs"]
mod tests;
