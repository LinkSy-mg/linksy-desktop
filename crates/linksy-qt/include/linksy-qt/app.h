// The application itself: QApplication rather than QGuiApplication, because KDE's desktop
// style for Qt Quick (qqc2-desktop-style) draws with QStyle, which needs it.
#pragma once

#include <QApplication>
#include <QClipboard>
#include <QIcon>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickStyle>
#include <QQuickWindow>
#include <QUrl>

#include "cxx-qt-lib/qstring.h"
#include "linksy-qt/video.h"

inline int run_app()
{
    int argc = 1;
    static char name[] = "linksy-qt";
    static char *argv[] = { name, nullptr };
    // The video item for other devices' screens draws with OpenGL
    QQuickWindow::setGraphicsApi(QSGRendererInterface::OpenGL);
    QApplication app(argc, argv);
    QApplication::setApplicationName(QStringLiteral("Linksy"));
    QApplication::setOrganizationDomain(QStringLiteral("diary.mg"));
    // The one "Linksy" in the menu (linksy-open picks this window on Plasma)
    QApplication::setDesktopFileName(QStringLiteral("mg.diary.linksy.Linksy"));
    QApplication::setWindowIcon(QIcon::fromTheme(QStringLiteral("mg.diary.linksy.Linksy")));
    // Plasma sets its own style; elsewhere, KDE's desktop style when it's installed.
    if (qEnvironmentVariableIsEmpty("QT_QUICK_CONTROLS_STYLE")) {
        QQuickStyle::setStyle(QStringLiteral("org.kde.desktop"));
        QQuickStyle::setFallbackStyle(QStringLiteral("Fusion"));
    }
    QQmlApplicationEngine engine;
    linksy_video::engine() = &engine;
    engine.rootContext()->setContextProperty(QStringLiteral("canShowScreens"), linksy_video::prepare());
    // cxx-qt puts a QML module's files under qrc:/qt/qml/<module path>/
    engine.load(QUrl(QStringLiteral("qrc:/qt/qml/mg/diary/linksy/qml/Main.qml")));
    if (engine.rootObjects().isEmpty()) {
        qCritical("linksy-qt: the window's QML didn't load (see the errors above)");
        return 1;
    }
    const int code = app.exec();
    stop_stream();
    return code;
}

inline QString clipboard_text()
{
    return QApplication::clipboard()->text();
}

inline void set_clipboard_text(const QString &text)
{
    QApplication::clipboard()->setText(text);
}

// Development aid: save the newest window (a dialog page over the main one), toolbar and
// all, as an image
inline bool save_window(const QString &path)
{
    const auto windows = QGuiApplication::topLevelWindows();
    for (auto it = windows.rbegin(); it != windows.rend(); ++it) {
        auto *quick = qobject_cast<QQuickWindow *>(*it);
        if (quick && quick->isVisible())
            return quick->grabWindow().save(path);
    }
    return false;
}
