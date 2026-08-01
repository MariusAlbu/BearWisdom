use super::{relative_named_imports, supertype_head};

#[test]
fn supertype_head_strips_generic_args() {
    assert_eq!(supertype_head("TestingLibraryMatchers<any, T>"), "TestingLibraryMatchers");
    assert_eq!(supertype_head("QueryObserverBaseResult"), "QueryObserverBaseResult");
}

#[test]
fn relative_named_imports_keeps_relative_named_only() {
    let src = "\
import {type TestingLibraryMatchers} from './matchers'\n\
import {Foo, Bar as Baz} from '../shared'\n\
import DefaultThing from './default'\n\
import * as NS from './namespace'\n\
import {Something} from 'aria-query'\n\
export {Other} from './other'\n";
    let got = relative_named_imports(src);
    // Relative named imports kept; `type` modifier and `as` rename reduced to the
    // local binding; default/namespace and bare-package imports dropped.
    assert!(got.contains(&("./matchers".to_string(), vec!["TestingLibraryMatchers".to_string()])));
    assert!(got.contains(&("../shared".to_string(), vec!["Foo".to_string(), "Baz".to_string()])));
    assert!(got.contains(&("./other".to_string(), vec!["Other".to_string()])));
    assert!(!got.iter().any(|(s, _)| s == "aria-query"));
    assert!(!got.iter().any(|(s, _)| s == "./default" || s == "./namespace"));
}
