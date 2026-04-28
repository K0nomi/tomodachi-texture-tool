"""
TTT2 - Zen Browser First Launch Experience
Cross-platform translucent + blurred window over the desktop wallpaper.

Layers (mirrors how Firefox/Zen does it):
  1. HTML/body transparent (handled in onboarding.html)
  2. Web engine paints with alpha (pywebview transparent=True)
  3. OS-native blur material on the host window (this file)
"""

import os
import sys
import ctypes
import threading
import time

import webview

# Sharp text on Windows HiDPI displays
if sys.platform == "win32":
    try:
        ctypes.windll.shcore.SetProcessDpiAwareness(2)
    except Exception:
        try:
            ctypes.windll.user32.SetProcessDPIAware()
        except Exception:
            pass


# ─── BLUR BACKENDS ──────────────────────────────────────────────────────────

def _apply_blur_windows(window):
    """Mica/Acrylic on Win11 22H2+; legacy accent-policy acrylic on Win10."""
    try:
        hwnd = int(window.native.Handle)
    except Exception:
        hwnd = ctypes.windll.user32.FindWindowW(None, "Zen Browser")
    if not hwnd:
        print("[blur] could not resolve HWND")
        return

    build = sys.getwindowsversion().build
    dwm = ctypes.windll.dwmapi

    class MARGINS(ctypes.Structure):
        _fields_ = [("cxLeftWidth", ctypes.c_int),
                    ("cxRightWidth", ctypes.c_int),
                    ("cyTopHeight", ctypes.c_int),
                    ("cyBottomHeight", ctypes.c_int)]

    # Extend frame across the entire client area so DWM composites the backdrop
    dwm.DwmExtendFrameIntoClientArea(hwnd, ctypes.byref(MARGINS(-1, -1, -1, -1)))

    # Match titlebar to dark mode (no-op when frameless, harmless)
    DWMWA_USE_IMMERSIVE_DARK_MODE = 20
    dark = ctypes.c_int(1)
    dwm.DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE,
                              ctypes.byref(dark), ctypes.sizeof(dark))

    # Win11 22H2+: modern system backdrop (preferred — smoother than legacy)
    if build >= 22621:
        DWMWA_SYSTEMBACKDROP_TYPE = 38
        DWMSBT_TRANSIENTWINDOW = 3   # Acrylic
        backdrop = ctypes.c_int(DWMSBT_TRANSIENTWINDOW)
        if dwm.DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE,
                                     ctypes.byref(backdrop),
                                     ctypes.sizeof(backdrop)) == 0:
            print(f"[blur] DWM Acrylic backdrop applied (Win11 build {build})")
            return

    # Win10 / early Win11 fallback: undocumented SetWindowCompositionAttribute
    ACCENT_ENABLE_ACRYLICBLURBEHIND = 4

    class ACCENT_POLICY(ctypes.Structure):
        _fields_ = [("AccentState", ctypes.c_uint),
                    ("AccentFlags", ctypes.c_uint),
                    ("GradientColor", ctypes.c_uint),  # AABBGGRR
                    ("AnimationId", ctypes.c_uint)]

    class WINCOMPATTRDATA(ctypes.Structure):
        _fields_ = [("Attribute", ctypes.c_int),
                    ("Data", ctypes.c_void_p),
                    ("SizeOfData", ctypes.c_size_t)]

    accent = ACCENT_POLICY()
    accent.AccentState = ACCENT_ENABLE_ACRYLICBLURBEHIND
    accent.GradientColor = 0xC0100820  # ~75% opaque dark navy tint
    accent.AccentFlags = 0x20

    data = WINCOMPATTRDATA()
    data.Attribute = 19  # WCA_ACCENT_POLICY
    data.Data = ctypes.cast(ctypes.pointer(accent), ctypes.c_void_p)
    data.SizeOfData = ctypes.sizeof(accent)

    ctypes.windll.user32.SetWindowCompositionAttribute(hwnd, ctypes.pointer(data))
    print(f"[blur] Legacy acrylic accent-policy applied (build {build})")


def _apply_blur_macos(window):
    """Insert NSVisualEffectView behind the WKWebView for native vibrancy."""
    try:
        from AppKit import (
            NSVisualEffectView, NSColor,
            NSVisualEffectMaterialHUDWindow,
            NSVisualEffectBlendingModeBehindWindow,
            NSVisualEffectStateActive,
            NSViewWidthSizable, NSViewHeightSizable,
        )
    except ImportError:
        print("[blur] pyobjc-framework-Cocoa missing; pip install it for blur")
        return

    try:
        wkwebview = window.native
        nswindow = wkwebview.window()
        if nswindow is None:
            return

        nswindow.setOpaque_(False)
        nswindow.setBackgroundColor_(NSColor.clearColor())

        # WKWebView paints opaque white by default — turn that off
        try:
            wkwebview.setValue_forKey_(False, "drawsBackground")
        except Exception:
            pass

        content_view = nswindow.contentView()
        effect = NSVisualEffectView.alloc().initWithFrame_(content_view.bounds())
        effect.setMaterial_(NSVisualEffectMaterialHUDWindow)
        effect.setBlendingMode_(NSVisualEffectBlendingModeBehindWindow)
        effect.setState_(NSVisualEffectStateActive)
        effect.setAutoresizingMask_(NSViewWidthSizable | NSViewHeightSizable)

        # positioned:NSWindowBelow (0) so the WKWebView stays in front
        content_view.addSubview_positioned_relativeTo_(effect, 0, None)
        print("[blur] NSVisualEffectView installed behind WKWebView")
    except Exception as e:
        print(f"[blur] macOS error: {e}")


def _apply_blur_linux(window):
    """KDE/KWin blur via X11 atom. No reliable cross-DE option exists:
    GNOME/Mutter has no blur API at all, Wayland needs compositor-specific
    protocols. On unsupported setups this silently degrades to plain alpha."""
    try:
        from Xlib import display, Xatom
    except ImportError:
        print("[blur] python-xlib missing; pip install it for KDE blur")
        return

    try:
        gtk_window = window.native
        gdk_window = gtk_window.get_window() if hasattr(gtk_window, "get_window") else None
        if gdk_window is None:
            return
        try:
            xid = gdk_window.get_xid()
        except Exception:
            print("[blur] Wayland session — KDE blur atom not applicable")
            return

        d = display.Display()
        atom = d.intern_atom("_KDE_NET_WM_BLUR_BEHIND_REGION")
        win = d.create_resource_object("window", xid)
        win.change_property(atom, Xatom.CARDINAL, 32, [])  # empty = whole window
        d.sync()
        print("[blur] _KDE_NET_WM_BLUR_BEHIND_REGION set (effective on KWin)")
    except Exception as e:
        print(f"[blur] Linux error: {e}")


def apply_blur(window):
    if sys.platform == "win32":
        _apply_blur_windows(window)
    elif sys.platform == "darwin":
        _apply_blur_macos(window)
    elif sys.platform.startswith("linux"):
        _apply_blur_linux(window)


# ─── APP ────────────────────────────────────────────────────────────────────

def get_html_path():
    return os.path.join(os.path.dirname(os.path.abspath(__file__)), "onboarding.html")


def on_loaded(window):
    time.sleep(0.25)  # let the OS finish creating/showing the window
    apply_blur(window)


def main():
    html_path = get_html_path()
    if not os.path.exists(html_path):
        print(f"Error: Could not find {html_path}")
        sys.exit(1)

    window = webview.create_window(
        title="Zen Browser",
        url=html_path,
        width=960,
        height=640,
        resizable=False,
        frameless=True,
        easy_drag=True,
        transparent=True,           # alpha-aware host window + WebView
        background_color="#0f0820", # initial paint before HTML loads
        text_select=False,
    )

    window.events.loaded += lambda: threading.Thread(
        target=on_loaded, args=(window,), daemon=True
    ).start()

    webview.start(debug=False)


if __name__ == "__main__":
    main()
