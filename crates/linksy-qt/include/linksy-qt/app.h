// The application itself: QApplication rather than QGuiApplication, because KDE's desktop
// style for Qt Quick (qqc2-desktop-style) draws with QStyle, which needs it.
#pragma once

#include <QApplication>
#include <QClipboard>
#include <QDir>
#include <QFileInfo>
#include <QIcon>
#include <QLocalServer>
#include <QLocalSocket>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickStyle>
#include <QQuickWindow>
#include <QStringList>
#include <QUrl>
#include <QVariant>

#include "cxx-qt-lib/qstring.h"
#include "cxx-qt-lib/qstringlist.h"
#include "linksy-qt/video.h"

// One window per service: a second start (from the menu, or the service opening it for a
// second display) shows the window already open, and quits.
inline QString instance_name()
{
    QString runtime = qEnvironmentVariable("XDG_RUNTIME_DIR");
    if (runtime.isEmpty())
        runtime = QDir::tempPath();
    // Per service: development setups run several, each with its own LINKSY_SOCKET
    const QString service = qEnvironmentVariable("LINKSY_SOCKET");
    return runtime + QStringLiteral("/linksy-qt-%1.sock").arg(qHash(service), 0, 16);
}

// Files to send hand over too ("Send with Linksy" in Dolphin): a "file <path>" line each.
inline bool show_running(const QString &name, const QStringList &files)
{
    QLocalSocket other;
    other.connectToServer(name);
    if (!other.waitForConnected(500))
        return false;
    QByteArray message("show\n");
    for (const QString &file : files)
        message += "file " + file.toUtf8() + '\n';
    other.write(message);
    other.waitForBytesWritten(500);
    other.disconnectFromServer();
    return true;
}

// Paths or file:// links as given on the command line, the ones that exist.
inline QStringList local_files(const QStringList &args)
{
    QStringList files;
    for (const QString &arg : args) {
        const QUrl url = QUrl::fromUserInput(arg, QDir::currentPath(), QUrl::AssumeLocalFile);
        if (url.isLocalFile() && QFileInfo::exists(url.toLocalFile()))
            files << url.toLocalFile();
    }
    return files;
}

inline void show_windows(QQmlApplicationEngine &engine, const QStringList &files)
{
    for (QObject *root : engine.rootObjects()) {
        if (auto *window = qobject_cast<QQuickWindow *>(root)) {
            window->show();
            window->raise();
            window->requestActivate();
        }
        if (!files.isEmpty())
            QMetaObject::invokeMethod(root, "offerFiles", Q_ARG(QVariant, QVariant(files)));
    }
}

inline int run_app(const QStringList &args)
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
    const QString instance_path = instance_name();
    const QStringList files = local_files(args);
    if (show_running(instance_path, files))
        return 0;
    // Nobody answered: a socket left behind by a crash goes
    QLocalServer::removeServer(instance_path);
    QLocalServer instance;
    instance.listen(instance_path);
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
    // The other start says what it has, then hangs up
    QObject::connect(&instance, &QLocalServer::newConnection, [&]() {
        while (QLocalSocket *other = instance.nextPendingConnection()) {
            auto *received = new QByteArray;
            QObject::connect(other, &QLocalSocket::readyRead, other, [other, received]() { *received += other->readAll(); });
            QObject::connect(other, &QLocalSocket::disconnected, other, [other, received, &engine]() {
                *received += other->readAll();
                QStringList handed;
                for (const QByteArray &line : received->split('\n'))
                    if (line.startsWith("file "))
                        handed << QString::fromUtf8(line.mid(5));
                delete received;
                show_windows(engine, handed);
                other->deleteLater();
            });
        }
    });
    if (!files.isEmpty())
        show_windows(engine, files);
    const int code = app.exec();
    stop_stream();
    return code;
}

inline void quit_app()
{
    QCoreApplication::quit();
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
