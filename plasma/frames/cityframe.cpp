// B63 — see cityframe.h. Keep this file free of anything beyond QtCore/QtGui/
// QtQuick (no private headers), so it builds against stock Qt 6.12 packages.

#include "cityframe.h"

#include <QByteArray>
#include <QFile>
#include <QPair>
#include <QRect>
#include <QVector>
#include <QDebug>
#include <QSGSimpleTextureNode>
#include <QSGTexture>
#include <QQuickWindow>

#ifdef CITYLIVE_PARTIAL_UPLOAD
#include <rhi/qrhi.h>
#endif

#include <atomic>
#include <algorithm>
#include <utility>
#include <cstddef>
#include <cstring>

#include <fcntl.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>

using citylive::FrameHeader;
using citylive::RectTable;

namespace {

// B86: protocol 4 adds the rectangle damage table and `prevSeq`. Version 3
// (rows only, the `seq != seen + 1` skip rule) is still read, so a reader newer
// than the wallpaper keeps working until the wallpaper is updated too.
constexpr quint32 kVersion = 4;
constexpr quint32 kVersionV3 = 3;
// One buffer (B61): the writer marks it "being written" before it touches a
// pixel, so a reader that arrives mid-copy keeps its previous frame and takes
// the next one. Half the shared memory of the old double buffer.
constexpr int kBuffers = 1;
constexpr int kMaxDimension = 16384;
/// Must match `MAX_DAMAGE_SPANS` in `crates/present/src/shm.rs` (version 3 rows).
constexpr int kMaxDamageRows = 24;
/// `damage[i].count` when the writer copied the whole head.
constexpr quint32 kDamageFull = 0xFFFFu;

/// Acquire load: the writer releases the buffer and the header in this order,
/// so a reader that sees the new header also sees the pixels behind it.
inline quint64 loadAcquire(const quint64 *p)
{
    return __atomic_load_n(p, __ATOMIC_ACQUIRE);
}

inline quint32 loadAcquire32(const quint32 *p)
{
    return __atomic_load_n(p, __ATOMIC_ACQUIRE);
}

bool magicOk(const char *m)
{
    static const char kMagic[8] = {'C', 'L', 'I', 'S', 'O', 'F', 'R', '1'};
    return std::memcmp(m, kMagic, 8) == 0;
}

} // namespace

#ifdef CITYLIVE_PARTIAL_UPLOAD
// B61f — a texture the reader can update a band of rows at a time.
//
// The old path was `createTextureFromImage()` every frame: Qt allocates a
// fresh QRhiTexture, uploads all 14.7 MB of a head and destroys the previous
// one, thirty times a second per head — which is most of the ~4 % plasmashell
// was spending. Nothing about the *transport* needed that: the writer already
// tells us which rows it changed, and the reader already copies only those
// rows. This texture takes the same row list and uploads only those rows into a
// texture that lives as long as the node does.
//
// Qt's own QSGPlainTexture works exactly this way: it creates its QRhiTexture
// inside `commitTextureOperations()`, which the scene graph calls on the render
// thread just before it binds the texture, handing over the frame's resource
// update batch. `QImage::Format_RGB32` is B,G,R,X in memory, which is what
// `QRhiTexture::BGRA8` expects, so the pixels go up without a swizzle.
class CityTexture : public QSGTexture
{
public:
    explicit CityTexture(const QSize &size)
        : m_size(size)
    {
    }

    ~CityTexture() override { delete m_tex; }

    qint64 comparisonKey() const override
    {
        return static_cast<qint64>(reinterpret_cast<quintptr>(this));
    }
    QRhiTexture *rhiTexture() const override { return m_tex; }
    QSize textureSize() const override { return m_size; }
    bool hasAlphaChannel() const override { return false; }
    bool hasMipmaps() const override { return false; }

    /// The frame the render thread is about to show: its pixels, the rows that
    /// changed since the last one, and whether the texture needs every pixel.
    void setPending(const uchar *bits, int stride, const QVector<QRect> &rects, bool full)
    {
        m_bits = bits;
        m_stride = stride;
        // Accumulate: a frame that was copied into the image but never reached
        // `commitTextureOperations` (the item was not drawn) must still be
        // uploaded with the next one. A runaway list is just a full upload.
        if (full || m_full) {
            m_full = true;
            m_rects.clear();
        } else {
            m_rects += rects;
            if (m_rects.size() > 20000) {
                m_full = true;
                m_rects.clear();
            }
        }
    }

    /// Creation failed (no BGRA8 on this backend): the item falls back to the
    /// whole-image path instead of showing nothing.
    bool failed() const { return m_failed; }

    void commitTextureOperations(QRhi *rhi, QRhiResourceUpdateBatch *updates) override
    {
        if (!rhi || !updates || !m_bits || m_failed)
            return;
        if (!m_tex) {
            if (!rhi->isTextureFormatSupported(QRhiTexture::BGRA8)) {
                m_failed = true;
                qWarning("citylive frames: no BGRA8 on this backend, whole-image upload");
                return;
            }
            m_tex = rhi->newTexture(QRhiTexture::BGRA8, m_size, 1, {});
            if (!m_tex || !m_tex->create()) {
                qWarning("citylive frames: %dx%d BGRA8 texture failed, whole-image upload",
                         m_size.width(), m_size.height());
                delete m_tex;
                m_tex = nullptr;
                m_failed = true;
                return;
            }
            m_full = true; // a new texture has no pixels at all yet
            qInfo("citylive frames: row-aware %dx%d texture (BGRA8, %s)",
                  m_size.width(), m_size.height(),
                  rhi->driverInfo().deviceName.isEmpty() ? "rhi"
                                                         : qPrintable(rhi->driverInfo().deviceName));
        }
        quint64 bytes = 0;
        int rowsUp = 0;
        if (m_full) {
            QRhiTextureSubresourceUploadDescription d(m_bits, quint32(m_stride) * quint32(m_size.height()));
            d.setDataStride(quint32(m_stride));
            updates->uploadTexture(m_tex, QRhiTextureUploadDescription(QRhiTextureUploadEntry(0, 0, d)));
            bytes = quint64(m_stride) * quint64(m_size.height());
            rowsUp = m_size.height();
        } else {
            // B86: one upload entry per changed rectangle, each with its own
            // tightly packed copy of just that rectangle's pixels. (A pointer
            // into the image with the image's stride would make Qt copy
            // `height * stride` bytes per entry: a 1x1 raindrop would cost a
            // whole 10 KB row, and rain would stage more than full rows did.)
            QVector<QRhiTextureUploadEntry> entries;
            entries.reserve(m_rects.size());
            for (const QRect &r : std::as_const(m_rects)) {
                if (r.width() <= 0 || r.height() <= 0)
                    continue;
                const qsizetype rowBytes = qsizetype(r.width()) * 4;
                QByteArray px(rowBytes * r.height(), Qt::Uninitialized);
                char *out = px.data();
                const uchar *in = m_bits + qsizetype(r.y()) * m_stride + qsizetype(r.x()) * 4;
                for (int y = 0; y < r.height(); ++y)
                    std::memcpy(out + qsizetype(y) * rowBytes, in + qsizetype(y) * m_stride, size_t(rowBytes));
                QRhiTextureSubresourceUploadDescription d(px);
                d.setSourceSize(r.size());
                d.setDestinationTopLeft(r.topLeft());
                entries.append(QRhiTextureUploadEntry(0, 0, d));
                bytes += quint64(r.width()) * quint64(r.height()) * 4u;
                rowsUp += r.height();
            }
#ifdef CITYLIVE_BATCH_UPLOAD
            // All rectangles in one upload description, so Qt stages them in one
            // buffer. (Defined by CMake; `setEntries(first, last)` is the Qt 6
            // iterator overload. Without it each rectangle is its own upload.)
            if (!entries.isEmpty()) {
                QRhiTextureUploadDescription desc;
                desc.setEntries(entries.cbegin(), entries.cend());
                updates->uploadTexture(m_tex, desc);
            }
#else
            for (const QRhiTextureUploadEntry &e : std::as_const(entries))
                updates->uploadTexture(m_tex, QRhiTextureUploadDescription(e));
#endif
        }
        m_full = false;
        m_rects.clear();
        // CITYLIVE_FRAMES_STATS: what this head actually sends to the GPU, so
        // the claim "only the damaged rows are uploaded" is measured, not
        // argued. Off by default: a wallpaper has no business logging.
        static const bool stats = qEnvironmentVariableIsSet("CITYLIVE_FRAMES_STATS");
        if (stats) {
            static quint64 accBytes = 0;
            static quint64 accRows = 0;
            static quint64 accFrames = 0;
            static QElapsedTimer since;
            if (!since.isValid())
                since.start();
            accBytes += bytes;
            accRows += quint64(rowsUp);
            if (++accFrames % 300 == 0) {
                const double secs = since.restart() / 1000.0;
                qInfo("citylive frames: %dx%d %.1f MB/s, %.0f rect rows of %d a frame",
                      m_size.width(), m_size.height(), accBytes / (1024.0 * 1024.0) / secs,
                      double(accRows) / double(accFrames), m_size.height());
                accBytes = 0;
                accRows = 0;
            }
        }
    }

private:
    QSize m_size;
    QRhiTexture *m_tex = nullptr;
    const uchar *m_bits = nullptr;
    int m_stride = 0;
    QVector<QRect> m_rects;
    bool m_full = true;
    bool m_failed = false;
};
#endif // CITYLIVE_PARTIAL_UPLOAD

CityFrame::CityFrame(QQuickItem *parent)
    : QQuickItem(parent)
{
    setFlag(ItemHasContents, true);
    // Precise, and polled at a quarter of the frame interval: a coarse timer at
    // exactly the writer's rate beats against it and skips or doubles frames,
    // which reads as stutter (the owner: "everything moves kinda stuttery"). The
    // poll itself is one atomic load.
    m_timer.setTimerType(Qt::PreciseTimer);
    m_timer.setInterval(250);
    connect(&m_timer, &QTimer::timeout, this, &CityFrame::tick);
    m_sinceFrame.start();
    m_sinceTry.start();
    if (m_active)
        m_timer.start();
}

CityFrame::~CityFrame()
{
    m_timer.stop();
    QMutexLocker lock(&m_mapMutex);
    closeLocked();
}

void CityFrame::setPath(const QString &path)
{
    QString p = path;
    // Accept both a plain path and a file:// URL.
    if (p.startsWith(QLatin1String("file://")))
        p = p.mid(7);
    {
        QMutexLocker lock(&m_mapMutex);
        if (p == m_path)
            return;
        closeLocked();
        m_needOpen = true;
        m_seen = 0;
        m_haveFrame = false;
        m_image = QImage();
        m_imageFilled = false;
        m_path = p;
    }
    emit pathChanged();
    setStatus(NoFile);
    if (m_active)
        tick();
}

void CityFrame::setActive(bool active)
{
    if (active == m_active)
        return;
    m_active = active;
    if (active) {
        m_timer.start();
        tick();
    } else {
        m_timer.stop();
    }
    emit activeChanged();
}


void CityFrame::setStatus(Status s)
{
    if (s == m_status)
        return;
    m_status = s;
    emit statusChanged();
}

void CityFrame::setTimerInterval()
{
    // Poll at the rate the writer says it publishes; a slower rate would add
    // latency, a faster one would just burn CPU. With nothing mapped, check
    // four times a second for a wallpaper that is not running yet.
    int fps = m_fps;
    if (fps <= 0 || fps > 240)
        fps = 30;
    const int ms = qBound(4, 1000 / (fps * 4), 250);
    if (m_timer.interval() != ms)
        m_timer.setInterval(ms);
}

qint64 CityFrame::staleAfterMs() const
{
    int fps = m_fps;
    if (fps <= 0 || fps > 240)
        fps = 30;
    // Well beyond a frame interval: an idle city writes nothing for seconds and
    // that is normal (D71: zero work when nothing moves), so only call it stale
    // once it has been quiet for a while.
    return qMax<qint64>(2000, 8000 / fps);
}

// --- mapping ---------------------------------------------------------------

bool CityFrame::openLocked()
{
    closeLocked();
    if (m_path.isEmpty())
        return false;
    const QByteArray name = QFile::encodeName(m_path);
    const int fd = ::open(name.constData(), O_RDONLY | O_CLOEXEC);
    if (fd < 0)
        return false;
    struct stat st;
    // The v3 part of the header is the smaller of the two layouts; a version 4
    // file is checked against the full rectangle table below.
    if (::fstat(fd, &st) != 0 || st.st_size < static_cast<off_t>(sizeof(FrameHeader))) {
        ::close(fd);
        return false;
    }
    void *map = ::mmap(nullptr, static_cast<size_t>(st.st_size), PROT_READ, MAP_SHARED, fd, 0);
    ::close(fd);
    if (map == MAP_FAILED)
        return false;

    const FrameHeader *h = static_cast<const FrameHeader *>(map);
    const size_t len = static_cast<size_t>(st.st_size);
    const quint32 w = h->width;
    const quint32 ht = h->height;
    bool ok = magicOk(h->magic)
        && (h->version == kVersion || h->version == kVersionV3)
        && h->buffers == static_cast<quint32>(kBuffers)
        && w > 0 && ht > 0 && w <= kMaxDimension && ht <= kMaxDimension
        && h->stride >= w * 4u
        && h->headerBytes >= sizeof(FrameHeader) && h->headerBytes <= len
        && (h->version == kVersionV3 || h->headerBytes >= citylive::kRectsEnd);
    for (int i = 0; ok && i < kBuffers; ++i) {
        const quint64 off = h->buf[i].offset;
        const quint32 bytes = h->buf[i].bytes;
        ok = off >= h->headerBytes && off <= len && bytes >= (h->stride * ht)
            && static_cast<size_t>(off) + bytes <= len;
    }
    if (!ok) {
        ::munmap(map, len);
        return false;
    }

    m_map = static_cast<uchar *>(map);
    m_len = len;
    m_version = static_cast<int>(h->version);
    m_dev = static_cast<quint64>(st.st_dev);
    m_ino = static_cast<quint64>(st.st_ino);
    m_needOpen = false;
    // A recreated file (or a restarted wallpaper) counts its frames from zero:
    // remembering the old number would mean never taking a new frame again.
    m_seen = 0;
    m_haveFrame = false;
    // The rate the writer advertises; the GUI thread applies it to the timer.
    m_headerFps = static_cast<int>(h->fps);
    return true;
}

void CityFrame::closeLocked()
{
    if (m_map) {
        ::munmap(m_map, m_len);
        m_map = nullptr;
        m_len = 0;
    }
}

bool CityFrame::outOfDateLocked() const
{
    if (!m_map)
        return true;
    struct stat st;
    const QByteArray name = QFile::encodeName(m_path);
    if (::stat(name.constData(), &st) != 0)
        return true; // gone
    return static_cast<quint64>(st.st_dev) != m_dev
        || static_cast<quint64>(st.st_ino) != m_ino
        || static_cast<size_t>(st.st_size) != m_len;
}

// --- the poll --------------------------------------------------------------

void CityFrame::tick()
{
    if (!m_active)
        return;

    bool mapped = false;
    bool want = false;
    bool fpsChangedNow = false;
    bool haveFrame = false;
    {
        QMutexLocker lock(&m_mapMutex);
        mapped = m_map != nullptr;
        haveFrame = m_haveFrame;
        if (mapped && outOfDateLocked()) {
            // Replaced or resized: re-map (and show the old frame until the new
            // one arrives).
            m_needOpen = true;
            mapped = false;
        }
        if (!mapped) {
            m_needOpen = true;
        } else {
            const FrameHeader *h = reinterpret_cast<const FrameHeader *>(m_map);
            const quint64 seq = loadAcquire(&h->seq);
            if (seq != 0 && seq != m_seen)
                want = true;
            if (m_headerFps != m_fps) {
                m_fps = m_headerFps;
                fpsChangedNow = true;
            }
        }
    }

    // QTimer belongs to this thread; the render thread only reports the rate.
    // Set the poll rate on every mapped tick, not only when the rate changes:
    // the timer starts (and after a remap restarts) at 250 ms, and a writer
    // advertising the default 30 fps never "changed" it, so the head was polled
    // four times a second and rain moved ~6x/s on screen (the owner's video, B86).
    if (mapped)
        setTimerInterval();
    if (fpsChangedNow)
        emit fpsChanged();

    if (!mapped) {
        // No file yet: look four times a second, not thirty, and keep asking the
        // render thread to (re)open it.
        if (m_timer.interval() != 250)
            m_timer.setInterval(250);
        if (m_sinceTry.elapsed() < 250)
            return;
        m_sinceTry.restart();
        setStatus(NoFile);
        m_dirty.store(true);
        update();
        return;
    }

    if (want || m_dirty.load()) {
        m_dirty.store(true);
        update(); // the copy happens on the render thread
        return;
    }
    const bool live = haveFrame && m_sinceFrame.elapsed() < staleAfterMs();
    setStatus(live ? Live : Stale);
}

// --- the frame -------------------------------------------------------------

bool CityFrame::refresh()
{
    // Only this thread maps or unmaps, so the pointer stays valid for the whole
    // function once it has been taken under the lock.
    uchar *map = nullptr;
    size_t len = 0;
    quint64 seen = 0;
    int version = 0;
    {
        QMutexLocker lock(&m_mapMutex);
        if (m_needOpen && !m_path.isEmpty())
            openLocked(); // a failure is reported by tick() as NoFile
        map = m_map;
        len = m_len;
        seen = m_seen;
        version = m_version;
    }
    if (!map)
        return false;

    const FrameHeader *h = reinterpret_cast<const FrameHeader *>(map);
    const quint64 seq = loadAcquire(&h->seq);
    if (seq == 0 || seq == seen)
        return false;
    const quint32 published = loadAcquire32(&h->published);
    if (published >= static_cast<quint32>(kBuffers))
        return false;
    const quint64 want = 2 * seq;
    if (loadAcquire(&h->buf[published].seq) != want)
        return false; // the writer is inside this buffer: keep the old frame

    const int w = static_cast<int>(h->width);
    const int ht = static_cast<int>(h->height);
    const int stride = static_cast<int>(h->stride);
    const uchar *src = map + h->buf[published].offset;
    if (h->buf[published].offset + static_cast<quint64>(stride) * ht > len)
        return false;

    if (m_image.width() != w || m_image.height() != ht) {
        // The QImage wraps m_pixels, so drop it before the buffer is replaced.
        m_image = QImage();
        // One spare row: see m_pixels.
        m_pixels.assign(static_cast<size_t>(w) * 4u * (static_cast<size_t>(ht) + 1u), 0);
        m_image = QImage(m_pixels.data(), w, ht, static_cast<qsizetype>(w) * 4, QImage::Format_RGB32);
        m_imageFilled = false;
    }
    if (m_image.isNull())
        return false;
    uchar *const dst = m_image.bits();
    const size_t dstStride = static_cast<size_t>(m_image.bytesPerLine());
    // B61/B86: copy only what the writer says it changed. The image is kept
    // between frames, so everything outside the damage is already correct: an
    // idle head copies nothing, and rain copies its drops.
    //
    // The table describes *this* publish against the one before it, and there
    // is no history in it. So the incremental copy is only right when the image
    // is exactly that one publish behind: version 4 names the publish
    // (`prevSeq`), version 3 only has the frame numbers, so a gap in them means
    // a frame went by unseen (B84 r3: the freeze). Anything else (never
    // filled, a torn frame dropped, a skipped publish, a count or rectangle
    // that makes no sense) copies the whole head, so a bad header can never
    // leave stale pixels.
    const bool v4 = version >= static_cast<int>(kVersion);
    bool behind;
    if (v4)
        behind = loadAcquire(&h->buf[published].prevSeq) != seen;
    else
        behind = seen != 0 && seq != seen + 1;
    bool full = !m_imageFilled || behind;
    QVector<QRect> rects;
    if (!full && v4) {
        const RectTable *t = reinterpret_cast<const RectTable *>(map + citylive::kRectsOffset);
        const quint32 count = loadAcquire32(&t->count);
        if (count == citylive::kRectsFull || count > citylive::kMaxDamageRects) {
            full = true;
        } else {
            rects.reserve(static_cast<int>(count));
            for (quint32 k = 0; k < count; ++k) {
                // A torn table can hold anything: clamp, and skip empties. (The
                // sequence check after the copy throws the frame away anyway.)
                const quint32 x0 = qMin<quint32>(t->rects[k].x0, static_cast<quint32>(w));
                const quint32 y0 = qMin<quint32>(t->rects[k].y0, static_cast<quint32>(ht));
                const quint32 x1 = qMin<quint32>(t->rects[k].x1, static_cast<quint32>(w));
                const quint32 y1 = qMin<quint32>(t->rects[k].y1, static_cast<quint32>(ht));
                if (x1 > x0 && y1 > y0)
                    rects.append(QRect(static_cast<int>(x0), static_cast<int>(y0),
                                       static_cast<int>(x1 - x0), static_cast<int>(y1 - y0)));
            }
        }
    } else if (!full) {
        // Version 3: whole-row bands become full-width rectangles.
        const FrameHeader::Damage &d = h->damage[published];
        if (d.count == kDamageFull || static_cast<int>(d.count) > kMaxDamageRows) {
            full = true;
        } else {
            // count == 0 means the writer changed no row: nothing is copied.
            for (quint32 k = 0; k < d.count; ++k) {
                const int y0 = static_cast<int>(qMin<quint32>(d.rows[k].y0, static_cast<quint32>(ht)));
                const int y1 = static_cast<int>(qMin<quint32>(d.rows[k].y1, static_cast<quint32>(ht)));
                if (y1 > y0)
                    rects.append(QRect(0, y0, w, y1 - y0));
            }
        }
    }
    if (full) {
        for (int y = 0; y < ht; ++y)
            std::memcpy(dst + static_cast<size_t>(y) * dstStride, src + static_cast<size_t>(y) * stride,
                        static_cast<size_t>(w) * 4);
        m_imageFilled = true;
    } else {
        for (int i = 0; i < rects.size(); ++i) {
            const QRect &r = rects[i];
            const size_t bytes = static_cast<size_t>(r.width()) * 4;
            for (int y = r.top(); y <= r.bottom(); ++y)
                std::memcpy(dst + static_cast<size_t>(y) * dstStride + static_cast<size_t>(r.left()) * 4,
                            src + static_cast<size_t>(y) * stride + static_cast<size_t>(r.left()) * 4, bytes);
        }
    }
    // What the render thread hands to the texture. A frame that named nothing
    // (the writer changed nothing on this head) leaves both empty, and the
    // texture already holds the picture: nothing is uploaded at all.
    m_fullCopy = full;
    m_rects = full ? QVector<QRect>() : rects;

    if (loadAcquire(&h->buf[published].seq) != want) {
        // Torn: a writer got in while we copied, so the image now holds pixels
        // from two frames. Drop the frame *and* make the next one a full copy,
        // so a partial copy can never leave a torn image on screen.
        m_imageFilled = false;
        return false;
    }

    // Hand the frame over to the GUI thread's bookkeeping. No signals from here:
    // updatePaintNode runs on the render thread, and `status` is a GUI property.
    {
        QMutexLocker lock(&m_mapMutex);
        m_seen = seq;
        m_haveFrame = true;
        m_sinceFrame.restart();
    }
    // Nothing copied means nothing to show that is not already on screen.
    return full || !rects.isEmpty();
}

QSGNode *CityFrame::updatePaintNode(QSGNode *oldNode, UpdatePaintNodeData *)
{
    QQuickWindow *win = window();
    if (!win)
        return oldNode;

    auto *node = static_cast<QSGSimpleTextureNode *>(oldNode);
    if (!node) {
        node = new QSGSimpleTextureNode;
        node->setOwnsTexture(true);
        node->setFiltering(QSGTexture::Nearest);
    }

    bool upload = false;
    if (m_dirty.exchange(false))
        upload = refresh();

#ifdef CITYLIVE_PARTIAL_UPLOAD
    // B61f: if the backend cannot take a BGRA8 texture, drop ours and use the
    // whole-image path for good (a texture that never appeared is worse than a
    // slow one).
    if (m_texLive) {
        auto *t = static_cast<CityTexture *>(node->texture());
        if (!t || t->failed()) {
            m_texLive = false;
            node->setTexture(nullptr);
            upload = !m_image.isNull();
        }
    }
#endif

    if (upload && !m_image.isNull()) {
#ifdef CITYLIVE_PARTIAL_UPLOAD
        // One texture per head size, kept by the node. A frame hands it the
        // rows that changed; a new one gets the whole image.
        if (!m_texLive || node->texture()->textureSize() != m_image.size()) {
            node->setTexture(new CityTexture(m_image.size())); // deletes the old
            m_texLive = true;
        }
        static_cast<CityTexture *>(node->texture())
            ->setPending(m_image.constBits(), int(m_image.bytesPerLine()), m_rects, m_fullCopy);
#else
        // B61: without the row-aware texture this is a full upload every frame
        // (Qt's public API only takes a whole QImage); what it still buys is
        // that the CPU copy above is damage-sized.
        // The image wraps our own buffer: hand Qt a copy it can keep.
        if (QSGTexture *tex = win->createTextureFromImage(m_image.copy())) {
            tex->setFiltering(QSGTexture::Nearest);
            node->setTexture(tex);
        }
#endif
    }
    node->setRect(boundingRect());
    return node;
}
