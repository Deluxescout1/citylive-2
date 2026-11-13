import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as Workspace from 'resource:///org/gnome/shell/ui/workspace.js';
import * as WorkspaceThumbnail from 'resource:///org/gnome/shell/ui/workspaceThumbnail.js';
import GLib from 'gi://GLib';

function isWallpaperWindow(win) {
    if (!win) return false;
    const appId = win.get_gtk_application_id?.() || win.get_wm_class?.() || '';
    const title = win.get_title?.() || '';
    return appId === 'org.citylive.Iso' || title.startsWith('citylive-iso-wallpaper');
}

function findMonitorForWindow(win) {
    const title = win.get_title() || '';
    const match = title.match(/^citylive-iso-wallpaper:(\d+):([^:]+):(-?\d+):(-?\d+):(\d+)x(\d+)/);
    if (match) {
        const idx = parseInt(match[1], 10);
        const name = match[2];
        const x = parseInt(match[3], 10);
        const y = parseInt(match[4], 10);

        // 1. Try connector name
        let mon = Main.layoutManager.monitors.find(m => m.connector === name);
        if (mon) return mon;

        // 2. Try coordinates
        mon = Main.layoutManager.monitors.find(m => m.x === x && m.y === y);
        if (mon) return mon;

        // 3. Try index
        if (idx >= 0 && idx < Main.layoutManager.monitors.length) {
            return Main.layoutManager.monitors[idx];
        }
    }

    // Fallback: win.get_monitor()
    const winMonIdx = win.get_monitor?.() ?? -1;
    if (winMonIdx >= 0 && winMonIdx < Main.layoutManager.monitors.length) {
        return Main.layoutManager.monitors[winMonIdx];
    }
    return Main.layoutManager.primaryMonitor || Main.layoutManager.monitors[0];
}

export default class CityLiveIsoExtension extends Extension {
    enable() {
        this._managedWindows = new Map();

        // 1. Filter out from Overview workspace thumbnails
        this._origThumbnailIsOverview = WorkspaceThumbnail.WorkspaceThumbnail.prototype._isOverviewWindow;
        WorkspaceThumbnail.WorkspaceThumbnail.prototype._isOverviewWindow = function (win) {
            const metaWin = win.get_meta_window ? win.get_meta_window() : win.meta_window;
            if (metaWin && isWallpaperWindow(metaWin)) {
                return false;
            }
            return this._origThumbnailIsOverview.call(this, win);
        }.bind(this);

        // 2. Filter out from Overview window picker (Workspace)
        this._origWorkspaceIsOverview = Workspace.Workspace.prototype._isOverviewWindow;
        Workspace.Workspace.prototype._isOverviewWindow = function (metaWin) {
            if (metaWin && isWallpaperWindow(metaWin)) {
                return false;
            }
            return this._origWorkspaceIsOverview.call(this, metaWin);
        }.bind(this);

        // 3. Filter out from Dash
        if (Main.overview.dash && typeof Main.overview.dash._redisplay === 'function') {
            this._origDashRedisplay = Main.overview.dash._redisplay.bind(Main.overview.dash);
            Main.overview.dash._redisplay = function () {
                this._origDashRedisplay();
                for (const item of Main.overview.dash._box.get_children()) {
                    const app = item.child?._delegate?.app;
                    if (app && (app.get_id() === 'org.citylive.Iso.desktop' || app.get_id() === 'org.citylive.Iso')) {
                        Main.overview.dash._box.remove_child(item);
                        item.destroy();
                    }
                }
            }.bind(this);
            Main.overview.dash._redisplay();
        }

        // 4. Watch for window creation and mapping
        this._windowCreatedId = global.display.connect('window-created', (display, win) => {
            this._watchWindow(win);
        });

        this._monitorsChangedId = Main.layoutManager.connect('monitors-changed', () => {
            this._updatePositions();
        });

        this._mapId = global.window_manager.connect('map', (wm, actor) => {
            if (actor && actor.meta_window) {
                this._checkAndManage(actor.meta_window);
            }
        });

        for (const actor of global.get_window_actors()) {
            if (actor && actor.meta_window) {
                this._watchWindow(actor.meta_window);
            }
        }

        console.log('[CityLive Iso] Extension enabled (B19b multi-monitor)');
    }

    _watchWindow(win) {
        if (isWallpaperWindow(win)) {
            this._checkAndManage(win);
            return;
        }

        const id1 = win.connect('notify::title', () => {
            if (isWallpaperWindow(win)) {
                this._checkAndManage(win);
            }
        });
        const id2 = win.connect('notify::wm-class', () => {
            if (isWallpaperWindow(win)) {
                this._checkAndManage(win);
            }
        });
        const id3 = win.connect('notify::gtk-application-id', () => {
            if (isWallpaperWindow(win)) {
                this._checkAndManage(win);
            }
        });

        const unmanagedId = win.connect('unmanaged', () => {
            win.disconnect(id1);
            win.disconnect(id2);
            win.disconnect(id3);
            win.disconnect(unmanagedId);
            this._unmanageWindow(win);
        });
    }

    _checkAndManage(win) {
        if (!win || this._managedWindows.has(win)) {
            return;
        }

        const title = win.get_title?.();
        // If title is not set yet, wait for it
        if (!title) {
            const titleId = win.connect('notify::title', () => {
                try {
                    win.disconnect(titleId);
                } catch (_) {}
                this._checkAndManage(win);
            });
            return;
        }

        if (!isWallpaperWindow(win)) {
            return;
        }

        const actor = win.get_compositor_private();
        if (actor && actor.get_first_child()) {
            this._manageWindow(win, actor);
            return;
        }

        let retries = 0;
        const timerId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 50, () => {
            retries++;
            if (this._managedWindows.has(win)) {
                return GLib.SOURCE_REMOVE;
            }
            const a = win.get_compositor_private();
            if (a && a.get_first_child()) {
                this._manageWindow(win, a);
                return GLib.SOURCE_REMOVE;
            }
            if (retries > 60) {
                return GLib.SOURCE_REMOVE;
            }
            return GLib.SOURCE_CONTINUE;
        });
    }

    _manageWindow(win, actor) {
        if (this._managedWindows.has(win)) {
            return;
        }

        const monitor = findMonitorForWindow(win);
        const originalParent = actor.get_parent();

        if (originalParent) {
            originalParent.remove_child(actor);
        }
        Main.layoutManager._backgroundGroup.add_child(actor);
        try {
            Main.layoutManager._backgroundGroup.set_child_at_index(actor, -1);
        } catch (_) {}

        try {
            if (typeof win.move_to_monitor === 'function') {
                win.move_to_monitor(monitor.index);
            }
        } catch (e) {
            console.log(`[CityLive Iso] move_to_monitor error: ${e}`);
        }
        try {
            if (typeof win.move_frame === 'function') {
                win.move_frame(true, monitor.x, monitor.y);
            }
        } catch (e) {
            console.log(`[CityLive Iso] move_frame error: ${e}`);
        }

        actor.visible = true;
        actor.reactive = false;
        actor.can_focus = false;
        actor.set_position(monitor.x, monitor.y);
        actor.set_size(monitor.width, monitor.height);

        const posId = actor.connect('notify::position', () => {
            if (actor.x !== monitor.x || actor.y !== monitor.y) {
                actor.set_position(monitor.x, monitor.y);
            }
        });
        const winPosId = win.connect('notify::position', () => {
            if (actor.x !== monitor.x || actor.y !== monitor.y) {
                actor.set_position(monitor.x, monitor.y);
            }
        });

        // Hide window from Alt-Tab and Overview
        let origSkipDescriptor = Object.getOwnPropertyDescriptor(win, 'skip_taskbar');
        let origIsSkip = typeof win.is_skip_taskbar === 'function' ? win.is_skip_taskbar.bind(win) : null;

        Object.defineProperty(win, 'skip_taskbar', {
            get: () => true,
            configurable: true,
        });
        win.is_skip_taskbar = () => true;

        const unmanagedId = win.connect('unmanaged', () => {
            this._unmanageWindow(win);
        });

        this._managedWindows.set(win, {
            actor,
            originalParent,
            unmanagedId,
            posId,
            winPosId,
            origSkipDescriptor,
            origIsSkip,
            monitor,
        });

        // Hide native background actors so live wallpaper shines through
        this._updateBackgroundVisibility();

        // Purge any clones from overview thumbnails and workspaces
        this._removeWindowClones(win);

        const frameRect = win.get_frame_rect ? win.get_frame_rect() : null;
        console.log(`[CityLive Iso] Window '${win.get_title()}' frameRect=(${frameRect?.x}, ${frameRect?.y}, ${frameRect?.width}, ${frameRect?.height}) managed for monitor '${monitor.connector || monitor.index}' at (${monitor.x}, ${monitor.y}) ${monitor.width}x${monitor.height}`);
        console.log(`[CityLive Iso] _backgroundGroup has ${Main.layoutManager._backgroundGroup.get_n_children()} children:`);
        for (const c of Main.layoutManager._backgroundGroup.get_children()) {
            console.log(`  child ${c}: pos=(${c.x}, ${c.y}) size=${c.width}x${c.height} vis=${c.visible}`);
        }
    }

    _updatePositions() {
        for (const [win, record] of this._managedWindows.entries()) {
            if (record.actor) {
                const monitor = findMonitorForWindow(win);
                record.monitor = monitor;
                record.actor.set_position(monitor.x, monitor.y);
                record.actor.set_size(monitor.width, monitor.height);
            }
        }
    }

    _updateBackgroundVisibility() {
        const hasManaged = this._managedWindows.size > 0;
        const bgChildren = Main.layoutManager._backgroundGroup.get_children();
        for (const child of bgChildren) {
            let isOurActor = false;
            for (const record of this._managedWindows.values()) {
                if (record.actor === child) {
                    isOurActor = true;
                    break;
                }
            }
            if (isOurActor) {
                child.visible = true;
            } else {
                child.visible = !hasManaged;
            }
        }
    }

    _removeWindowClones(win) {
        // 1. Remove from workspace thumbnails
        try {
            const controls = Main.overview._overview?.controls;
            const thumbnailsBox = controls?._thumbnailsBox || controls?._workspacesThumbnails;
            const thumbnails = thumbnailsBox?._thumbnails || [];
            for (const t of thumbnails) {
                if (typeof t._doRemoveWindow === 'function') {
                    t._doRemoveWindow(win);
                }
            }
        } catch (e) {
            console.log(`[CityLive Iso] Error removing from thumbnails: ${e}`);
        }

        // 2. Remove from workspaces
        try {
            const controls = Main.overview._overview?.controls;
            const workspacesDisplay = controls?._workspacesDisplay;
            const views = workspacesDisplay?._workspacesViews || [];
            for (const v of views) {
                const workspaces = v._workspaces || [];
                for (const ws of workspaces) {
                    if (typeof ws._doRemoveWindow === 'function') {
                        ws._doRemoveWindow(win);
                    }
                }
            }
        } catch (e) {
            console.log(`[CityLive Iso] Error removing from workspaces: ${e}`);
        }

        // 3. Remove from dash
        try {
            if (Main.overview.dash) {
                Main.overview.dash._redisplay();
            }
        } catch (_) {}
    }

    _unmanageWindow(win, restoring = false) {
        const record = this._managedWindows.get(win);
        if (!record) return;

        if (record.unmanagedId) {
            try {
                win.disconnect(record.unmanagedId);
            } catch (_) {}
        }
        if (record.winPosId) {
            try {
                win.disconnect(record.winPosId);
            } catch (_) {}
        }
        if (record.posId && record.actor) {
            try {
                record.actor.disconnect(record.posId);
            } catch (_) {}
        }
        if (restoring) {
            if (record.origSkipDescriptor) {
                try {
                    Object.defineProperty(win, 'skip_taskbar', record.origSkipDescriptor);
                } catch (_) {}
            }
            if (record.origIsSkip && typeof win.is_skip_taskbar === 'function') {
                win.is_skip_taskbar = record.origIsSkip;
            }

            if (record.actor) {
                try {
                    const parent = record.actor.get_parent();
                    if (parent === Main.layoutManager._backgroundGroup) {
                        Main.layoutManager._backgroundGroup.remove_child(record.actor);
                        if (record.originalParent) {
                            record.originalParent.add_child(record.actor);
                        } else if (Main.layoutManager._windowGroup) {
                            Main.layoutManager._windowGroup.add_child(record.actor);
                        }
                    }
                    record.actor.reactive = true;
                    record.actor.can_focus = true;
                } catch (_) {}
            }
        }

        this._managedWindows.delete(win);
        this._updateBackgroundVisibility();
        console.log(`[CityLive Iso] Window unmanaged`);
    }

    disable() {
        if (this._origThumbnailIsOverview) {
            WorkspaceThumbnail.WorkspaceThumbnail.prototype._isOverviewWindow = this._origThumbnailIsOverview;
            this._origThumbnailIsOverview = null;
        }

        if (this._origWorkspaceIsOverview) {
            Workspace.Workspace.prototype._isOverviewWindow = this._origWorkspaceIsOverview;
            this._origWorkspaceIsOverview = null;
        }

        if (this._origDashRedisplay) {
            Main.overview.dash._redisplay = this._origDashRedisplay;
            this._origDashRedisplay = null;
            try {
                Main.overview.dash._redisplay();
            } catch (_) {}
        }

        if (this._windowCreatedId) {
            global.display.disconnect(this._windowCreatedId);
            this._windowCreatedId = null;
        }

        if (this._monitorsChangedId) {
            Main.layoutManager.disconnect(this._monitorsChangedId);
            this._monitorsChangedId = null;
        }

        if (this._mapId) {
            global.window_manager.disconnect(this._mapId);
            this._mapId = null;
        }

        const windows = Array.from(this._managedWindows.keys());
        for (const win of windows) {
            this._unmanageWindow(win, true);
        }
        this._managedWindows.clear();
        this._updateBackgroundVisibility();

        console.log('[CityLive Iso] Extension disabled');
    }
}
