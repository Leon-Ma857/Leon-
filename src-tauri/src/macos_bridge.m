#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#import <CoreGraphics/CoreGraphics.h>
#import <Foundation/Foundation.h>
#import <ImageIO/ImageIO.h>
#import <Vision/Vision.h>

#include <math.h>
#include <pthread.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static const int64_t kLumaInjectedEventMarker = 0x4C554D41;
static const CGKeyCode kLumaKeyCodeC = 8;
static const CGKeyCode kLumaKeyCodeSpace = 49;
static const CGKeyCode kLumaKeyCodeTab = 48;
static const CGKeyCode kLumaKeyCodeEscape = 53;

static char *luma_strdup_ns(NSString *str) {
    if (!str) {
        return NULL;
    }
    const char *utf8 = str.UTF8String;
    if (!utf8) {
        return NULL;
    }
    return strdup(utf8);
}

static void luma_set_err(char **out_err, NSString *msg) {
    if (out_err) {
        *out_err = luma_strdup_ns(msg ?: @"未知错误");
    }
}

void luma_mac_free_string(char *s) {
    if (s) {
        free(s);
    }
}

static void run_on_main_sync(dispatch_block_t block) {
    if ([NSThread isMainThread]) {
        block();
    } else {
        dispatch_sync(dispatch_get_main_queue(), block);
    }
}

// Convert global AppKit coordinates (bottom-left primary origin) <-> Quartz coordinates (top-left primary origin).
static NSScreen *screen_for_cg_point(CGPoint pt, CGFloat *out_primary_h) {
    NSArray<NSScreen *> *screens = [NSScreen screens];
    if (screens.count == 0) {
        if (out_primary_h) {
            *out_primary_h = 1080.0;
        }
        return nil;
    }
    CGFloat primaryH = screens.firstObject.frame.size.height;
    if (out_primary_h) {
        *out_primary_h = primaryH;
    }
    for (NSScreen *scr in screens) {
        NSRect f = scr.frame;
        CGFloat top = primaryH - (f.origin.y + f.size.height);
        CGRect cgFrame = CGRectMake(f.origin.x, top, f.size.width, f.size.height);
        if (CGRectContainsPoint(cgFrame, pt)) {
            return scr;
        }
    }
    return screens.firstObject;
}

static NSScreen *screen_for_physical_point(int32_t phys_x, int32_t phys_y, CGFloat *out_primary_h) {
    NSArray<NSScreen *> *screens = [NSScreen screens];
    if (screens.count == 0) {
        if (out_primary_h) {
            *out_primary_h = 1080.0;
        }
        return nil;
    }
    CGFloat primaryH = screens.firstObject.frame.size.height;
    if (out_primary_h) {
        *out_primary_h = primaryH;
    }
    for (NSScreen *scr in screens) {
        NSRect f = scr.frame;
        CGFloat scale = fmax(scr.backingScaleFactor, 1.0);
        CGFloat top = primaryH - (f.origin.y + f.size.height);
        double leftP = f.origin.x * scale;
        double topP = top * scale;
        double rightP = (f.origin.x + f.size.width) * scale;
        double bottomP = (top + f.size.height) * scale;
        if ((double)phys_x >= leftP && (double)phys_x < rightP &&
            (double)phys_y >= topP && (double)phys_y < bottomP) {
            return scr;
        }
    }
    return screens.firstObject;
}

static void current_cursor_physical(int32_t *out_x, int32_t *out_y) {
    __block int32_t px = 100;
    __block int32_t py = 100;
    CGEventRef ev = CGEventCreate(NULL);
    CGPoint cgPt = ev ? CGEventGetLocation(ev) : CGPointZero;
    if (ev) {
        CFRelease(ev);
    }
    run_on_main_sync(^{
        CGFloat primaryH = 0;
        NSScreen *scr = screen_for_cg_point(cgPt, &primaryH);
        CGFloat scale = scr ? fmax(scr.backingScaleFactor, 1.0) : 1.0;
        px = (int32_t)llround(cgPt.x * scale);
        py = (int32_t)llround(cgPt.y * scale);
    });
    if (out_x) *out_x = px;
    if (out_y) *out_y = py;
}

bool luma_mac_get_cursor_point(int32_t *out_x, int32_t *out_y) {
    @autoreleasepool {
        current_cursor_physical(out_x, out_y);
        return true;
    }
}

bool luma_mac_work_area_for_point(
    int32_t phys_x,
    int32_t phys_y,
    int32_t *left,
    int32_t *top,
    int32_t *right,
    int32_t *bottom,
    float *scale_out
) {
    @autoreleasepool {
        __block BOOL ok = NO;
        __block int32_t wl = 0, wt = 0, wr = 0, wb = 0;
        __block float ws = 1.0f;
        run_on_main_sync(^{
            CGFloat primaryH = 0;
            NSScreen *scr = screen_for_physical_point(phys_x, phys_y, &primaryH);
            if (!scr) {
                return;
            }
            NSRect vf = scr.visibleFrame;
            CGFloat scale = fmax(scr.backingScaleFactor, 1.0);
            CGFloat cgTop = primaryH - (vf.origin.y + vf.size.height);
            CGFloat cgBottom = primaryH - vf.origin.y;
            wl = (int32_t)llround(vf.origin.x * scale);
            wt = (int32_t)llround(cgTop * scale);
            wr = (int32_t)llround((vf.origin.x + vf.size.width) * scale);
            wb = (int32_t)llround(cgBottom * scale);
            ws = (float)scale;
            ok = YES;
        });
        if (!ok) {
            return false;
        }
        if (left) *left = wl;
        if (top) *top = wt;
        if (right) *right = wr;
        if (bottom) *bottom = wb;
        if (scale_out) *scale_out = ws;
        return true;
    }
}

// ==================== Apple Vision OCR ====================

static VNRecognizeTextRequest *create_vision_ocr_request(BOOL cpu_only) {
    VNRecognizeTextRequest *request = [[VNRecognizeTextRequest alloc] initWithCompletionHandler:nil];
    request.recognitionLevel = VNRequestTextRecognitionLevelAccurate;
    request.usesLanguageCorrection = YES;
    request.usesCPUOnly = cpu_only;
    if (@available(macOS 13.0, *)) {
        request.automaticallyDetectsLanguage = YES;
    } else {
        request.recognitionLanguages = @[
            @"zh-Hans", @"zh-Hant", @"en-US", @"ja-JP", @"ko-KR", @"fr-FR", @"de-DE", @"es-ES"
        ];
    }
    return request;
}

bool luma_mac_ocr_png(
    const uint8_t *bytes,
    size_t len,
    char **out_json,
    char **out_err
) {
    @autoreleasepool {
        if (!bytes || len == 0) {
            luma_set_err(out_err, @"OCR 输入图像数据为空");
            return false;
        }
        NSData *data = [NSData dataWithBytes:bytes length:len];
        CGImageSourceRef src = CGImageSourceCreateWithData((__bridge CFDataRef)data, NULL);
        if (!src) {
            luma_set_err(out_err, @"无法解码 PNG 图像");
            return false;
        }
        CGImageRef cgImg = CGImageSourceCreateImageAtIndex(src, 0, NULL);
        CFRelease(src);
        if (!cgImg) {
            luma_set_err(out_err, @"无法从 PNG 构建 CGImage");
            return false;
        }

        VNRecognizeTextRequest *request = create_vision_ocr_request(NO);
        NSError *vnErr = nil;
        VNImageRequestHandler *handler = [[VNImageRequestHandler alloc] initWithCGImage:cgImg options:@{}];
        BOOL ok = [handler performRequests:@[request] error:&vnErr];
        if (!ok) {
            // Virtualized or headless macOS runners without Metal/ANE require CPU fallback.
            VNRecognizeTextRequest *cpuRequest = create_vision_ocr_request(YES);
            vnErr = nil;
            if ([handler performRequests:@[cpuRequest] error:&vnErr]) {
                request = cpuRequest;
                ok = YES;
            }
        }
        CGImageRelease(cgImg);

        if (!ok) {
            NSString *desc = vnErr.localizedDescription ?: @"未知 Vision 错误";
            luma_set_err(out_err, [NSString stringWithFormat:@"Apple Vision OCR 识别失败: %@", desc]);
            return false;
        }

        NSMutableArray<NSDictionary *> *items = [NSMutableArray array];
        for (VNRecognizedTextObservation *obs in request.results) {
            VNRecognizedText *cand = [obs topCandidates:1].firstObject;
            if (!cand) {
                continue;
            }
            NSString *trimmed = [cand.string stringByTrimmingCharactersInSet:[NSCharacterSet whitespaceAndNewlineCharacterSet]];
            if (trimmed.length == 0) {
                continue;
            }
            CGRect box = obs.boundingBox;
            double x = fmax(0.0, fmin(1.0, box.origin.x));
            double w = fmax(0.005, fmin(1.0 - x, box.size.width));
            double y = fmax(0.0, fmin(1.0, 1.0 - box.origin.y - box.size.height));
            double h = fmax(0.005, fmin(1.0 - y, box.size.height));
            [items addObject:@{
                @"x": @(x),
                @"y": @(y),
                @"w": @(w),
                @"h": @(h),
                @"text": trimmed
            }];
        }

        NSError *jsonErr = nil;
        NSData *jsonData = [NSJSONSerialization dataWithJSONObject:items options:0 error:&jsonErr];
        if (!jsonData) {
            luma_set_err(out_err, [NSString stringWithFormat:@"序列化 OCR 结果失败: %@", jsonErr.localizedDescription]);
            return false;
        }
        NSString *jsonStr = [[NSString alloc] initWithData:jsonData encoding:NSUTF8StringEncoding];
        if (out_json) {
            *out_json = luma_strdup_ns(jsonStr ?: @"[]");
        }
        return true;
    }
}

// ==================== Non-polluting Selection Capture ====================

typedef struct {
    NSArray<NSArray<NSDictionary *> *> *savedItems;
} LumaPasteboardSnapshot;

static NSArray<NSArray<NSDictionary *> *> *snapshot_pasteboard_items(NSPasteboard *pb) {
    NSMutableArray *result = [NSMutableArray array];
    for (NSPasteboardItem *item in pb.pasteboardItems) {
        NSMutableArray<NSDictionary *> *entries = [NSMutableArray array];
        for (NSPasteboardType type in item.types) {
            if ([type isEqualToString:@"org.nspasteboard.TransientType"] ||
                [type isEqualToString:@"org.nspasteboard.ConcealedType"] ||
                [type isEqualToString:@"org.nspasteboard.AutoGeneratedType"]) {
                continue;
            }
            NSData *data = [item dataForType:type];
            if (data) {
                [entries addObject:@{ @"type": type, @"data": data }];
            }
        }
        if (entries.count > 0) {
            [result addObject:entries];
        }
    }
    return result;
}

static void restore_pasteboard_items(NSPasteboard *pb, NSArray<NSArray<NSDictionary *> *> *snapshot) {
    [pb clearContents];
    if (snapshot.count > 0) {
        NSMutableArray<NSPasteboardItem *> *itemsToWrite = [NSMutableArray arrayWithCapacity:snapshot.count];
        for (NSUInteger idx = 0; idx < snapshot.count; idx++) {
            NSArray<NSDictionary *> *entries = snapshot[idx];
            NSPasteboardItem *newItem = [[NSPasteboardItem alloc] init];
            for (NSDictionary *entry in entries) {
                [newItem setData:entry[@"data"] forType:entry[@"type"]];
            }
            if (idx == 0) {
                NSData *empty = [NSData data];
                [newItem setData:empty forType:@"org.nspasteboard.TransientType"];
                [newItem setData:empty forType:@"org.nspasteboard.AutoGeneratedType"];
            }
            [itemsToWrite addObject:newItem];
        }
        [pb writeObjects:itemsToWrite];
    } else {
        NSPasteboardItem *marker = [[NSPasteboardItem alloc] init];
        NSData *empty = [NSData data];
        [marker setData:empty forType:@"org.nspasteboard.TransientType"];
        [marker setData:empty forType:@"org.nspasteboard.AutoGeneratedType"];
        [pb writeObjects:@[marker]];
    }
}

static BOOL check_accessibility_with_prompt(void) {
    if (AXIsProcessTrusted()) {
        return YES;
    }
    NSDictionary *opts = @{ (__bridge id)kAXTrustedCheckOptionPrompt: @YES };
    return AXIsProcessTrustedWithOptions((__bridge CFDictionaryRef)opts);
}

static void post_synthetic_cmd_c(void) {
    CGEventSourceRef src = CGEventSourceCreate(kCGEventSourceStateHIDSystemState);
    CGEventRef keyDown = CGEventCreateKeyboardEvent(src, kLumaKeyCodeC, true);
    CGEventRef keyUp = CGEventCreateKeyboardEvent(src, kLumaKeyCodeC, false);
    if (keyDown) {
        CGEventSetFlags(keyDown, kCGEventFlagMaskCommand);
        CGEventSetIntegerValueField(keyDown, kCGEventSourceUserData, kLumaInjectedEventMarker);
        CGEventPost(kCGHIDEventTap, keyDown);
        CFRelease(keyDown);
    }
    if (keyUp) {
        CGEventSetFlags(keyUp, kCGEventFlagMaskCommand);
        CGEventSetIntegerValueField(keyUp, kCGEventSourceUserData, kLumaInjectedEventMarker);
        CGEventPost(kCGHIDEventTap, keyUp);
        CFRelease(keyUp);
    }
    if (src) {
        CFRelease(src);
    }
}

bool luma_mac_capture_selected_text(
    int32_t *out_x,
    int32_t *out_y,
    char **out_text,
    char **out_err
) {
    @autoreleasepool {
        if (!check_accessibility_with_prompt()) {
            luma_set_err(out_err, @"需要在「系统设置 → 隐私与安全性 → 辅助功能」中授予 Luma 权限后才能全局划词取词。");
            return false;
        }

        int32_t cx = 100, cy = 100;
        current_cursor_physical(&cx, &cy);
        if (out_x) *out_x = cx;
        if (out_y) *out_y = cy;

        NSPasteboard *pb = [NSPasteboard generalPasteboard];
        NSArray<NSArray<NSDictionary *> *> *snapshot = snapshot_pasteboard_items(pb);
        NSInteger changeBefore = pb.changeCount;

        post_synthetic_cmd_c();

        BOOL changed = NO;
        for (int i = 0; i < 24; i++) {
            usleep(15000);
            if (pb.changeCount != changeBefore) {
                changed = YES;
                break;
            }
        }

        if (!changed) {
            luma_set_err(out_err, @"未检测到选中的文字，请先用鼠标划选文字后再按快捷键");
            return false;
        }

        NSString *copied = [pb stringForType:NSPasteboardTypeString];
        restore_pasteboard_items(pb, snapshot);

        NSString *trimmed = [copied stringByTrimmingCharactersInSet:[NSCharacterSet whitespaceAndNewlineCharacterSet]];
        if (!trimmed || trimmed.length == 0) {
            luma_set_err(out_err, @"选中的内容不包含可翻译文本");
            return false;
        }
        if (out_text) {
            *out_text = luma_strdup_ns(trimmed);
        }
        return true;
    }
}

// ==================== Smart Single-Key CGEventTap ====================

typedef void (*LumaMacMouseCallback)(int32_t kind, int32_t x, int32_t y, int32_t fg_pid, bool is_self);
typedef bool (*LumaMacKeyCallback)(uint32_t vk, bool has_mods, bool is_own_injected, int32_t fg_pid, bool is_self);

static pthread_mutex_t g_tap_lock = PTHREAD_MUTEX_INITIALIZER;
static CFMachPortRef g_event_tap = NULL;
static CFRunLoopRef g_tap_runloop = NULL;
static pthread_t g_tap_thread;
static BOOL g_tap_running = NO;
static LumaMacMouseCallback g_mouse_cb = NULL;
static LumaMacKeyCallback g_key_cb = NULL;
static pid_t g_self_pid = 0;

static void current_frontmost_app(int32_t *out_pid, bool *out_is_self) {
    NSRunningApplication *front = [NSWorkspace sharedWorkspace].frontmostApplication;
    pid_t pid = front ? front.processIdentifier : 0;
    if (out_pid) *out_pid = (int32_t)pid;
    if (out_is_self) *out_is_self = (pid != 0 && pid == g_self_pid);
}

static uint32_t map_mac_keycode_to_vk(CGKeyCode kc) {
    switch (kc) {
        case kLumaKeyCodeSpace: return 0x20; // VK_SPACE
        case kLumaKeyCodeTab: return 0x09;   // VK_TAB
        default: return (uint32_t)(0x10000 + kc);
    }
}

static CGEventRef smart_tap_callback(
    CGEventTapProxy proxy,
    CGEventType type,
    CGEventRef event,
    void *user_info
) {
    (void)proxy;
    (void)user_info;
    @autoreleasepool {
        if (type == kCGEventTapDisabledByTimeout || type == kCGEventTapDisabledByUserInput) {
            if (g_event_tap) {
                CGEventTapEnable(g_event_tap, true);
            }
            return event;
        }

        int32_t fgPid = 0;
        bool isSelf = false;
        current_frontmost_app(&fgPid, &isSelf);

        if (type == kCGEventLeftMouseDown || type == kCGEventLeftMouseUp ||
            type == kCGEventRightMouseDown || type == kCGEventOtherMouseDown) {
            LumaMacMouseCallback mcb = g_mouse_cb;
            if (mcb) {
                CGPoint cgPt = CGEventGetLocation(event);
                int32_t px = (int32_t)llround(cgPt.x);
                int32_t py = (int32_t)llround(cgPt.y);
                // Scale CGEvent point using the screen containing cgPt for unified physical pixels.
                CGFloat primaryH = 0;
                NSScreen *scr = screen_for_cg_point(cgPt, &primaryH);
                if (scr) {
                    CGFloat s = fmax(scr.backingScaleFactor, 1.0);
                    px = (int32_t)llround(cgPt.x * s);
                    py = (int32_t)llround(cgPt.y * s);
                }
                int32_t kind = (type == kCGEventLeftMouseDown) ? 0
                             : (type == kCGEventLeftMouseUp) ? 1 : 2;
                mcb(kind, px, py, fgPid, isSelf);
            }
            return event;
        }

        if (type == kCGEventKeyDown) {
            LumaMacKeyCallback kcb = g_key_cb;
            if (kcb) {
                CGKeyCode kc = (CGKeyCode)CGEventGetIntegerValueField(event, kCGKeyboardEventKeycode);
                int64_t marker = CGEventGetIntegerValueField(event, kCGEventSourceUserData);
                bool isOwnInjected = (marker == kLumaInjectedEventMarker);
                CGEventFlags flags = CGEventGetFlags(event);
                bool hasMods = (flags & (kCGEventFlagMaskCommand | kCGEventFlagMaskControl |
                                         kCGEventFlagMaskAlternate | kCGEventFlagMaskShift)) != 0;
                uint32_t vk = map_mac_keycode_to_vk(kc);
                if (kcb(vk, hasMods, isOwnInjected, fgPid, isSelf)) {
                    return NULL;
                }
            }
        }
    }
    return event;
}

typedef struct {
    pthread_mutex_t mu;
    pthread_cond_t cv;
    bool done;
    bool ok;
} LumaTapStartSync;

static void *smart_tap_thread_main(void *arg) {
    LumaTapStartSync *sync = (LumaTapStartSync *)arg;
    @autoreleasepool {
        CGEventMask mask = CGEventMaskBit(kCGEventLeftMouseDown)
                         | CGEventMaskBit(kCGEventLeftMouseUp)
                         | CGEventMaskBit(kCGEventRightMouseDown)
                         | CGEventMaskBit(kCGEventOtherMouseDown)
                         | CGEventMaskBit(kCGEventKeyDown);
        CFMachPortRef tap = CGEventTapCreate(
            kCGSessionEventTap,
            kCGHeadInsertEventTap,
            kCGEventTapOptionDefault,
            mask,
            smart_tap_callback,
            NULL
        );
        if (!tap) {
            pthread_mutex_lock(&sync->mu);
            sync->ok = false;
            sync->done = true;
            pthread_cond_signal(&sync->cv);
            pthread_mutex_unlock(&sync->mu);
            return NULL;
        }

        CFRunLoopSourceRef source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0);
        CFRunLoopRef rl = CFRunLoopGetCurrent();
        CFRunLoopAddSource(rl, source, kCFRunLoopCommonModes);
        CGEventTapEnable(tap, true);

        pthread_mutex_lock(&g_tap_lock);
        g_event_tap = tap;
        g_tap_runloop = rl;
        g_tap_running = YES;
        pthread_mutex_unlock(&g_tap_lock);

        pthread_mutex_lock(&sync->mu);
        sync->ok = true;
        sync->done = true;
        pthread_cond_signal(&sync->cv);
        pthread_mutex_unlock(&sync->mu);

        CFRunLoopRun();

        CFRunLoopRemoveSource(rl, source, kCFRunLoopCommonModes);
        CFRelease(source);
        CFMachPortInvalidate(tap);
        CFRelease(tap);

        pthread_mutex_lock(&g_tap_lock);
        g_event_tap = NULL;
        g_tap_runloop = NULL;
        g_tap_running = NO;
        pthread_mutex_unlock(&g_tap_lock);
    }
    return NULL;
}

void luma_mac_smart_hook_stop(void) {
    pthread_mutex_lock(&g_tap_lock);
    BOOL wasRunning = g_tap_running;
    CFRunLoopRef rl = g_tap_runloop;
    pthread_t th = g_tap_thread;
    g_mouse_cb = NULL;
    g_key_cb = NULL;
    if (rl) {
        CFRunLoopStop(rl);
    }
    pthread_mutex_unlock(&g_tap_lock);

    if (wasRunning) {
        pthread_join(th, NULL);
    }
}

bool luma_mac_smart_hook_start(
    LumaMacMouseCallback mouse_cb,
    LumaMacKeyCallback key_cb,
    char **out_err
) {
    @autoreleasepool {
        g_self_pid = getpid();
        luma_mac_smart_hook_stop();

        if (!check_accessibility_with_prompt()) {
            luma_set_err(out_err, @"启用单键划词需要在「系统设置 → 隐私与安全性 → 辅助功能 / 输入监控」中授予 Luma 权限。");
            return false;
        }

        pthread_mutex_lock(&g_tap_lock);
        g_mouse_cb = mouse_cb;
        g_key_cb = key_cb;
        pthread_mutex_unlock(&g_tap_lock);

        LumaTapStartSync sync;
        pthread_mutex_init(&sync.mu, NULL);
        pthread_cond_init(&sync.cv, NULL);
        sync.done = false;
        sync.ok = false;

        pthread_t th;
        if (pthread_create(&th, NULL, smart_tap_thread_main, &sync) != 0) {
            pthread_mutex_destroy(&sync.mu);
            pthread_cond_destroy(&sync.cv);
            luma_set_err(out_err, @"启动 macOS 事件监听线程失败");
            return false;
        }

        pthread_mutex_lock(&sync.mu);
        while (!sync.done) {
            pthread_cond_wait(&sync.cv, &sync.mu);
        }
        bool started = sync.ok;
        pthread_mutex_unlock(&sync.mu);
        pthread_mutex_destroy(&sync.mu);
        pthread_cond_destroy(&sync.cv);

        if (!started) {
            pthread_join(th, NULL);
            luma_set_err(out_err, @"创建 macOS 全局事件监听失败：请在「系统设置 → 隐私与安全性 → 辅助功能与输入监控」中授权 Luma。");
            return false;
        }

        pthread_mutex_lock(&g_tap_lock);
        g_tap_thread = th;
        pthread_mutex_unlock(&g_tap_lock);
        return true;
    }
}

// ==================== Floating Windows & Result Popover ====================

void luma_mac_configure_floating_window(void *ns_window) {
    if (!ns_window) return;
    NSWindow *win = (__bridge NSWindow *)ns_window;
    dispatch_async(dispatch_get_main_queue(), ^{
        win.collectionBehavior |=
            (NSWindowCollectionBehaviorMoveToActiveSpace | NSWindowCollectionBehaviorFullScreenAuxiliary);
    });
}

void luma_mac_configure_overlay_window(void *ns_window) {
    if (!ns_window) return;
    NSWindow *win = (__bridge NSWindow *)ns_window;
    dispatch_async(dispatch_get_main_queue(), ^{
        win.collectionBehavior |=
            (NSWindowCollectionBehaviorMoveToActiveSpace
             | NSWindowCollectionBehaviorFullScreenAuxiliary
             | NSWindowCollectionBehaviorStationary
             | NSWindowCollectionBehaviorIgnoresCycle);
        win.level = NSMainMenuWindowLevel + 1;
    });
}

void luma_mac_force_window_foreground(void *ns_window) {
    if (!ns_window) return;
    NSWindow *win = (__bridge NSWindow *)ns_window;
    dispatch_async(dispatch_get_main_queue(), ^{
        [[NSApplication sharedApplication] activateIgnoringOtherApps:YES];
        [win makeKeyAndOrderFront:nil];
    });
}

int32_t luma_mac_frontmost_external_pid(void) {
    @autoreleasepool {
        NSRunningApplication *front = [NSWorkspace sharedWorkspace].frontmostApplication;
        if (!front) return 0;
        pid_t pid = front.processIdentifier;
        if (pid == 0 || pid == getpid()) return 0;
        if ([front.bundleIdentifier isEqualToString:@"com.apple.dock"]) return 0;
        return (int32_t)pid;
    }
}

typedef void (*LumaMacDismissCallback)(void);

static BOOL g_popover_active = NO;
static __weak NSWindow *g_popover_window = nil;
static int32_t g_popover_source_pid = 0;
static uint64_t g_popover_activated_ms = 0;
static LumaMacDismissCallback g_popover_dismiss_cb = NULL;
static id g_workspace_observer = nil;
static id g_mouse_monitor = nil;

static uint64_t monotonic_ms(void) {
    return (uint64_t)(NSProcessInfo.processInfo.systemUptime * 1000.0);
}

static void uninstall_popover_monitors_on_main(void) {
    if (g_workspace_observer) {
        [[[NSWorkspace sharedWorkspace] notificationCenter] removeObserver:g_workspace_observer];
        g_workspace_observer = nil;
    }
    if (g_mouse_monitor) {
        [NSEvent removeMonitor:g_mouse_monitor];
        g_mouse_monitor = nil;
    }
}

static void ensure_popover_monitors_on_main(void) {
    if (!g_workspace_observer) {
        g_workspace_observer = [[[NSWorkspace sharedWorkspace] notificationCenter]
            addObserverForName:NSWorkspaceDidActivateApplicationNotification
                        object:nil
                         queue:[NSOperationQueue mainQueue]
                    usingBlock:^(NSNotification *note) {
                        if (!g_popover_active) return;
                        NSWindow *win = g_popover_window;
                        if (!win || !win.isVisible || win.level != NSNormalWindowLevel) return;
                        NSRunningApplication *app = note.userInfo[NSWorkspaceApplicationKey];
                        if (app && g_popover_source_pid > 0 && app.processIdentifier == g_popover_source_pid) {
                            dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(15 * NSEC_PER_MSEC)),
                                           dispatch_get_main_queue(), ^{
                                if (!g_popover_active) return;
                                NSWindow *w = g_popover_window;
                                if (!w || !w.isVisible || w.level != NSNormalWindowLevel) return;
                                NSRunningApplication *cur = [NSWorkspace sharedWorkspace].frontmostApplication;
                                if (cur && cur.processIdentifier == g_popover_source_pid) {
                                    [w orderFrontRegardless];
                                }
                            });
                        }
                    }];
    }
    if (!g_mouse_monitor) {
        NSEventMask mask = NSEventMaskLeftMouseDown | NSEventMaskRightMouseDown;
        g_mouse_monitor = [NSEvent addGlobalMonitorForEventsMatchingMask:mask
                                                                 handler:^(NSEvent * _Nonnull ev) {
            (void)ev;
            if (!g_popover_active) return;
            if (monotonic_ms() - g_popover_activated_ms < 250) return;
            NSWindow *win = g_popover_window;
            if (!win || !win.isVisible || win.level != NSNormalWindowLevel) return;
            NSPoint loc = [NSEvent mouseLocation];
            if (NSPointInRect(loc, win.frame)) return;
            NSRunningApplication *front = [NSWorkspace sharedWorkspace].frontmostApplication;
            pid_t fpid = front ? front.processIdentifier : 0;
            pid_t selfPid = getpid();
            BOOL inSourceOrPopup = (fpid == selfPid) || (g_popover_source_pid > 0 && fpid == g_popover_source_pid);
            if (!inSourceOrPopup) {
                return;
            }
            // Wait slightly so clicks hitting another app or menu bar take effect first if needed.
            LumaMacDismissCallback cb = g_popover_dismiss_cb;
            g_popover_active = NO;
            g_popover_source_pid = 0;
            if (cb) {
                dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
                    cb();
                });
            }
        }];
    }
}

void luma_mac_popover_deactivate(void) {
    dispatch_async(dispatch_get_main_queue(), ^{
        g_popover_active = NO;
        g_popover_source_pid = 0;
    });
}

void luma_mac_popover_activate(
    void *ns_window,
    int32_t source_pid,
    LumaMacDismissCallback on_dismiss
) {
    if (!ns_window) {
        luma_mac_popover_deactivate();
        return;
    }
    NSWindow *win = (__bridge NSWindow *)ns_window;
    dispatch_async(dispatch_get_main_queue(), ^{
        ensure_popover_monitors_on_main();
        g_popover_window = win;
        g_popover_source_pid = source_pid;
        g_popover_dismiss_cb = on_dismiss;
        g_popover_activated_ms = monotonic_ms();
        g_popover_active = YES;
    });
}

void luma_mac_popover_shutdown(void) {
    dispatch_async(dispatch_get_main_queue(), ^{
        g_popover_active = NO;
        g_popover_source_pid = 0;
        g_popover_dismiss_cb = NULL;
        g_popover_window = nil;
        uninstall_popover_monitors_on_main();
    });
}

// ==================== Pasteboard Files & Scroll Capture ====================

bool luma_mac_copy_files_to_pasteboard(
    const char *const *paths,
    size_t count,
    char **out_err
) {
    @autoreleasepool {
        if (!paths || count == 0) {
            luma_set_err(out_err, @"没有可复制的文件路径");
            return false;
        }
        NSMutableArray<NSURL *> *urls = [NSMutableArray arrayWithCapacity:count];
        for (size_t i = 0; i < count; i++) {
            if (!paths[i]) continue;
            NSString *p = [NSString stringWithUTF8String:paths[i]];
            if (p.length > 0) {
                [urls addObject:[NSURL fileURLWithPath:p]];
            }
        }
        if (urls.count == 0) {
            luma_set_err(out_err, @"没有有效的图片文件路径");
            return false;
        }
        NSPasteboard *pb = [NSPasteboard generalPasteboard];
        [pb clearContents];
        BOOL ok = [pb writeObjects:urls];
        if (!ok) {
            luma_set_err(out_err, @"写入系统多文件剪贴板失败");
            return false;
        }
        return true;
    }
}

void luma_mac_send_scroll_wheel_at(int32_t phys_x, int32_t phys_y, int32_t lines) {
    @autoreleasepool {
        __block CGPoint cgPt = CGPointMake(phys_x, phys_y);
        run_on_main_sync(^{
            CGFloat primaryH = 0;
            NSScreen *scr = screen_for_physical_point(phys_x, phys_y, &primaryH);
            CGFloat scale = scr ? fmax(scr.backingScaleFactor, 1.0) : 1.0;
            cgPt = CGPointMake((CGFloat)phys_x / scale, (CGFloat)phys_y / scale);
        });
        CGWarpMouseCursorPosition(cgPt);
        CGEventRef ev = CGEventCreateScrollWheelEvent(NULL, kCGScrollEventUnitLine, 1, lines);
        if (ev) {
            CGEventPost(kCGHIDEventTap, ev);
            CFRelease(ev);
        }
    }
}

bool luma_mac_is_escape_pressed(void) {
    return CGEventSourceKeyState(kCGEventSourceStateCombinedSessionState, kLumaKeyCodeEscape);
}
