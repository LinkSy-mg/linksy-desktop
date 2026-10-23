//! The C++ side: the application, and the clipboard (QML can't read it).

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("linksy-qt/app.h");
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        fn run_app() -> i32;
        fn clipboard_text() -> QString;
        fn set_clipboard_text(text: &QString);
        fn save_window(path: &QString) -> bool;
    }
}
