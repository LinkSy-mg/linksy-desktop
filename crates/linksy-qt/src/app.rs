//! The C++ side: the application, the clipboard (QML can't read it) and the video of
//! other devices' screens.

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("linksy-qt/app.h");
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;

        /// `args`: files to send, as paths or file:// links.
        fn run_app(args: &QStringList) -> i32;
        fn quit_app();
        fn clipboard_text() -> QString;
        fn set_clipboard_text(text: &QString);
        fn save_window(path: &QString) -> bool;

        fn play_stream(item: &QString, socket: &QString) -> QString;
        fn play_source(item: &QString, source: &QString) -> QString;
        fn stop_stream();
        fn stream_size() -> QString;
        fn send_control(json: &QString) -> bool;
    }
}
