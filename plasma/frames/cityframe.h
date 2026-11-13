// B63 — CityFrame: a QQuickItem that shows one shared-memory frame written by
// the CityLive Iso wallpaper.
//
// One file per head (`<shm-dir>/<output>.frame`), mmapped read-only here and
// published with a seqlock by the writer. There is no file watching, no image
// decoder and no CPU upscale: a QTimer polls the header's frame number at the
// rate the header advertises, the pixels are copied out of the mapping inside
// `updatePaintNode` (render thread) straight into a texture, and the GPU scales
// it with nearest-neighbour filtering.
//
// Nothing in here may crash plasmashell: every header field is validated and
// every offset is bounds-checked before it is used, and a file that appears,
// disappears or is replaced is handled by re-mapping (never by assuming).

#pragma once

#include <QElapsedTimer>
#include <QImage>
#include <QMutex>
#include <QPointer>
#include <QRect>
#include <QQuickItem>
#include <QTimer>

#include <atomic>
#include <vector>

#include <QtQml/qqmlregistration.h>

class QSGTexture;
class QQuickWindow;

namespace citylive {

/// The on-disk header. Offsets must match `crates/present/src/shm.rs`
/// (the static_asserts below are the check).
struct FrameHeader
{
    char magic[8];          // 0   "CLISOFR1"
    quint32 version;        // 8
    quint32 headerBytes;    // 12
    quint32 width;          // 16
    quint32 height;         // 20
    quint32 stride;         // 24  bytes per row
    quint32 scale;          // 28  integer upscale the screen applies
    quint32 screenW;        // 32
    quint32 screenH;        // 36
    quint32 fps;            // 40
    quint32 buffers;        // 44
    quint64 seq;            // 48  published frame number
    quint32 published;      // 56  which buffer holds it
    quint32 reserved;       // 60

    struct Buf {
        quint64 offset;     // file offset of the pixels
        quint64 seq;        // 2*frame complete, 2*frame+1 while being written
        quint32 bytes;      // stride * height
        quint32 pad0;
        quint64 prevSeq;    // 88: v4 (B86) `seq` of the publish before this one
                            //     (0 for the first); v3 files leave it 0
    } buf[2];               // 64

    // B61 (version 2): the rows each buffer last wrote. The reader keeps its
    // image between frames and copies only these rows, so its cost follows the
    // damage instead of the frame. `damageCount` == 0xFFFF means "all rows".
    struct Damage {
        quint32 count;      // 128 + i * 200
        quint32 pad0;
        struct Row {           // y0 .. y1, half open, rows of the head
            quint32 y0;
            quint32 y1;
        } rows[24];
    } damage[2];            // 128
};

static_assert(sizeof(FrameHeader::Buf) == 32, "buffer entries are 32 bytes each");
static_assert(sizeof(FrameHeader::Damage::Row) == 8, "a damage row span is two u32");
static_assert(sizeof(FrameHeader::Damage) == 200, "a damage entry is 200 bytes");
static_assert(offsetof(FrameHeader, version) == 8, "header layout");
static_assert(offsetof(FrameHeader, width) == 16, "header layout");
static_assert(offsetof(FrameHeader, stride) == 24, "header layout");
static_assert(offsetof(FrameHeader, scale) == 28, "header layout");
static_assert(offsetof(FrameHeader, fps) == 40, "header layout");
static_assert(offsetof(FrameHeader, buffers) == 44, "header layout");
static_assert(offsetof(FrameHeader, seq) == 48, "header layout");
static_assert(offsetof(FrameHeader, published) == 56, "header layout");
static_assert(offsetof(FrameHeader, buf) == 64, "header layout");
static_assert(offsetof(FrameHeader, buf) + sizeof(FrameHeader::Buf) == 96, "header layout");
static_assert(offsetof(FrameHeader, damage) == 128, "header layout");
static_assert(offsetof(FrameHeader, damage) + 2 * sizeof(FrameHeader::Damage) == 528, "header layout");
static_assert(offsetof(FrameHeader::Buf, prevSeq) == 24, "prev_seq sits at file offset 88");
static_assert(sizeof(FrameHeader) <= 4096, "the v3 header sits in one page");

// B86 (version 4): the rectangle damage table, at a fixed file offset after the
// two v3 row tables. Each publish names the rectangles it changed against the
// publish before it (`prevSeq`); the reader copies and uploads only those.
// Must match `OFF_RECTS`, `MAX_DAMAGE_RECTS`, `RECTS_FULL` in
// `crates/present/src/shm.rs`.
constexpr quint32 kRectsOffset = 576;
constexpr quint32 kMaxDamageRects = 3072;
constexpr quint32 kRectsFull = 0xFFFFFFFFu;

struct DamageRect
{
    quint32 x0;             // half open, pixels of the head
    quint32 y0;
    quint32 x1;
    quint32 y1;
};

struct RectTable
{
    quint32 count;          // 576  rectangles, or kRectsFull = the whole head
    quint32 pad0;
    DamageRect rects[kMaxDamageRects];  // 584
};

static_assert(sizeof(DamageRect) == 16, "a damage rectangle is four u32");
static_assert(sizeof(RectTable) == 8 + 16 * 3072, "rectangle table layout");
static_assert(kRectsOffset >= offsetof(FrameHeader, damage) + 2 * sizeof(FrameHeader::Damage),
              "the rectangle table follows the v3 row tables");
constexpr quint32 kRectsEnd = kRectsOffset + sizeof(RectTable);

} // namespace citylive

class CityFrame : public QQuickItem
{
    Q_OBJECT
    QML_ELEMENT

public:
    enum Status {
        NoFile = 0, // nothing mapped (no wallpaper running, or the file is bad)
        Live = 1,   // frames are arriving
        Stale = 2   // mapped, but nothing new for a while (the city is asleep)
    };
    Q_ENUM(Status)

    Q_PROPERTY(QString path READ path WRITE setPath NOTIFY pathChanged)
    Q_PROPERTY(bool active READ isActive WRITE setActive NOTIFY activeChanged)
    Q_PROPERTY(Status status READ status NOTIFY statusChanged)
    Q_PROPERTY(int fps READ fps NOTIFY fpsChanged)

    explicit CityFrame(QQuickItem *parent = nullptr);
    ~CityFrame() override;

    QString path() const { return m_path; }
    void setPath(const QString &path);
    bool isActive() const { return m_active; }
    void setActive(bool active);
    Status status() const { return m_status; }
    int fps() const { return m_fps; }

    QSGNode *updatePaintNode(QSGNode *oldNode, UpdatePaintNodeData *data) override;

signals:
    void pathChanged();
    void activeChanged();
    void statusChanged();
    void fpsChanged();

private slots:
    void tick();

private:
    void setStatus(Status s);
    /// Guarded by m_mapMutex: map and validate the file (or unmap on failure).
    bool openLocked();
    void closeLocked();
    bool outOfDateLocked() const;
    void setTimerInterval();
    /// Render thread: (re)map if needed and copy the newest frame into m_image.
    bool refresh();
    qint64 staleAfterMs() const;

    QString m_path;
    bool m_active = true;
    Status m_status = NoFile;
    int m_fps = 30;
    QTimer m_timer;

    // The mapping itself: created, read and destroyed only under this mutex.
    // The pixel copy runs outside it (only this class ever unmaps, and only on
    // the render thread).
    mutable QMutex m_mapMutex;
    uchar *m_map = nullptr;
    size_t m_len = 0;
    quint64 m_dev = 0;
    quint64 m_ino = 0;
    quint64 m_seen = 0;     // last frame number shown
    bool m_needOpen = true; // the render thread must (re)map before reading
    std::atomic<bool> m_dirty { false }; // a frame is waiting for the render thread
    int m_headerFps = 0;    // fps the writer advertises (applied by tick())

    QImage m_image;         // the frame as last copied, RGB32 (wraps m_pixels)
    /// B86: the image's pixels, one spare row long. A texture upload of one
    /// rectangle reads `height * stride` bytes from the rectangle's first pixel,
    /// which for a rectangle on the bottom rows ends one stride past the image.
    std::vector<uchar> m_pixels;
    int m_version = 0;      // the open file's protocol version (3 or 4)
    /// B61: has m_image ever been filled completely? Until it has, every frame
    /// copies the whole head (the untouched rows would otherwise be zeros).
    bool m_imageFilled = false;
    /// B61f/B86: the rectangles copied into m_image for the frame the render
    /// thread is about to upload (empty, with m_imageFilled set, means "nothing
    /// new": the texture already holds this picture and nothing is uploaded at
    /// all). A v3 file's row bands arrive here as full-width rectangles.
    QVector<QRect> m_rects;
    /// B61f: did the last refresh copy the whole head? (A new texture needs it.)
    bool m_fullCopy = true;
    /// B61f: is the node's texture one of ours (the row-aware `CityTexture`)?
    /// False falls back to a Qt texture created from the whole image.
    bool m_texLive = false;
    bool m_haveFrame = false;
    QElapsedTimer m_sinceFrame;
    QElapsedTimer m_sinceTry; // last attempt to open the file
};
