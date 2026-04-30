use super::*;

#[test]
fn synthetic_file_emits_qt_test_macros() {
    let file = synthesize_file();
    let names: Vec<&str> = file.symbols.iter().map(|s| s.name.as_str()).collect();
    for expected in &[
        "QCOMPARE", "QVERIFY", "QFAIL", "QFETCH",
        "QTRY_COMPARE", "QTRY_VERIFY", "QTRY_VERIFY_WITH_TIMEOUT",
        "QSKIP", "QEXPECT_FAIL", "QBENCHMARK",
    ] {
        assert!(
            names.contains(expected),
            "missing Qt test macro '{expected}'"
        );
    }
}

#[test]
fn synthetic_file_emits_qt_object_macros() {
    let file = synthesize_file();
    let names: Vec<&str> = file.symbols.iter().map(|s| s.name.as_str()).collect();
    for expected in &[
        "Q_OBJECT", "Q_GADGET", "Q_PROPERTY", "Q_INVOKABLE",
        "Q_SIGNALS", "Q_SLOTS", "Q_EMIT",
        "Q_DECLARE_METATYPE", "Q_DECLARE_FLAGS",
        "Q_DISABLE_COPY",
    ] {
        assert!(
            names.contains(expected),
            "missing Qt object-system macro '{expected}'"
        );
    }
}

#[test]
fn synthetic_file_emits_qt_signal_slot_markers() {
    let file = synthesize_file();
    let names: Vec<&str> = file.symbols.iter().map(|s| s.name.as_str()).collect();
    for expected in &["SIGNAL", "SLOT", "METHOD"] {
        assert!(
            names.contains(expected),
            "missing Qt signal/slot marker '{expected}'"
        );
    }
}

#[test]
fn synthetic_file_emits_qt_logging_functions() {
    let file = synthesize_file();
    let names: Vec<&str> = file.symbols.iter().map(|s| s.name.as_str()).collect();
    for expected in &[
        "qDebug", "qWarning", "qCritical", "qInfo", "qFatal",
        "qPrintable", "qUtf8Printable",
    ] {
        assert!(
            names.contains(expected),
            "missing Qt logging function '{expected}'"
        );
    }
}

#[test]
fn synthetic_file_emits_qt_typedefs() {
    let file = synthesize_file();
    let names: Vec<&str> = file.symbols.iter().map(|s| s.name.as_str()).collect();
    for expected in &[
        "qint32", "qint64", "quint32", "quint64", "qreal", "qsizetype", "QRgb",
    ] {
        assert!(
            names.contains(expected),
            "missing Qt typedef '{expected}'"
        );
    }
}

#[test]
fn synthetic_file_emits_qt_core_classes() {
    let file = synthesize_file();
    let names: Vec<&str> = file.symbols.iter().map(|s| s.name.as_str()).collect();
    for expected in &[
        "QObject", "QString", "QByteArray", "QVariant",
        "QList", "QVector", "QMap", "QHash", "QSet", "QPair",
        "QSharedPointer", "QScopedPointer", "QPointer",
        "QApplication", "QWidget", "QMainWindow",
    ] {
        assert!(
            names.contains(expected),
            "missing Qt core class '{expected}'"
        );
    }
}

#[test]
fn synthetic_file_path_is_external_prefixed() {
    let file = synthesize_file();
    assert!(
        file.path.starts_with("ext:"),
        "synthetic file must use ext: prefix; got {}",
        file.path,
    );
}

#[test]
fn synthetic_file_no_duplicate_qualified_names() {
    let file = synthesize_file();
    let mut seen = std::collections::HashSet::new();
    for sym in &file.symbols {
        let unique = seen.insert(sym.qualified_name.clone());
        assert!(
            unique,
            "duplicate qualified_name in qt synthetic file: {}",
            sym.qualified_name,
        );
    }
}
