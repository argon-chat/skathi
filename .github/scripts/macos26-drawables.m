#import <AppKit/AppKit.h>
#import <IOSurface/IOSurface.h>
#import <Metal/Metal.h>
#import <QuartzCore/QuartzCore.h>

@interface DrawableProbe : NSObject <NSApplicationDelegate, CAMetalDisplayLinkDelegate>
@property(copy) NSString *mode;
@property NSUInteger limit;
@property float latency;
@property(strong) NSWindow *window;
@property(strong) CAMetalLayer *layer;
@property(strong) id<MTLCommandQueue> queue;
@property(strong) CAMetalDisplayLink *link;
@property CFTimeInterval started;
@property NSUInteger updates;
@property NSUInteger submitted;
@end

@implementation DrawableProbe

- (void)encode:(id<CAMetalDrawable>)drawable source:(const char *)source {
    if (!drawable) {
        fprintf(stderr, "No drawable on %s at %.3fms\n", source, (CACurrentMediaTime() - self.started) * 1000);
        return;
    }
    NSUInteger index = ++self.submitted;
    IOSurfaceRef surface = drawable.texture.iosurface;
    IOSurfaceID sid = surface ? IOSurfaceGetID(surface) : 0;
    CFTimeInterval start = self.started;
    double at = (CACurrentMediaTime() - start) * 1000;
    printf("draw %s %.3f %lu sid=0x%x layer=%p\n", source, at, index, sid, self.layer);

    MTLRenderPassDescriptor *pass = [MTLRenderPassDescriptor renderPassDescriptor];
    pass.colorAttachments[0].texture = drawable.texture;
    pass.colorAttachments[0].loadAction = MTLLoadActionClear;
    pass.colorAttachments[0].storeAction = MTLStoreActionStore;
    pass.colorAttachments[0].clearColor = MTLClearColorMake((index % 200) / 255.0, 0.18, 0.36, 1);
    [drawable addPresentedHandler:^(id<MTLDrawable> presented) {
        double when = presented.presentedTime;
        printf("presented %s %.3f %lu 0x%x %.3f\n", source, when ? (when - start) * 1000 : 0,
               index, sid, when ? (when - start) * 1000 - at : 0);
    }];
    id<MTLCommandBuffer> commands = [self.queue commandBuffer];
    [[commands renderCommandEncoderWithDescriptor:pass] endEncoding];
    [commands presentDrawable:drawable];
    [commands commit];
}

- (void)metalDisplayLink:(CAMetalDisplayLink *)link needsUpdate:(CAMetalDisplayLinkUpdate *)update {
    @autoreleasepool {
        self.updates++;
        id<CAMetalDrawable> drawable = update.drawable;
        IOSurfaceRef surface = drawable.texture.iosurface;
        double at = CACurrentMediaTime();
        printf("update %.3f target=%.3f lead=%.3f sid=0x%x layer=%p\n", (at - self.started) * 1000,
               (update.targetPresentationTimestamp - self.started) * 1000,
               (update.targetPresentationTimestamp - at) * 1000, surface ? IOSurfaceGetID(surface) : 0,
               self.layer);
        if (![self.mode isEqualToString:@"idle"] || self.submitted == 0)
            [self encode:drawable source:"link"];
    }
}

- (void)attachLink {
    self.link = [[CAMetalDisplayLink alloc] initWithMetalLayer:self.layer];
    self.link.preferredFrameLatency = self.latency;
    self.link.delegate = self;
    [self.link addToRunLoop:NSRunLoop.mainRunLoop forMode:NSRunLoopCommonModes];
    printf("attach %.3f layer=%p maximum=%lu preferred=%.1f\n",
           (CACurrentMediaTime() - self.started) * 1000, self.layer,
           self.layer.maximumDrawableCount, self.link.preferredFrameLatency);
}

- (void)applicationDidFinishLaunching:(NSNotification *)notification {
    self.started = CACurrentMediaTime();
    self.window = [[NSWindow alloc] initWithContentRect:NSMakeRect(100, 100, 600, 400)
                                             styleMask:NSWindowStyleMaskTitled backing:NSBackingStoreBuffered defer:NO];
    self.window.title = @"Metal drawable diagnostics";
    self.layer = [CAMetalLayer layer];
    self.layer.device = MTLCreateSystemDefaultDevice();
    if (!self.layer.device) {
        fprintf(stderr, "No Metal GPU available\n");
        exit(2);
    }
    self.queue = [self.layer.device newCommandQueue];
    self.layer.pixelFormat = MTLPixelFormatBGRA8Unorm;
    self.layer.maximumDrawableCount = self.limit;
    self.layer.contentsScale = self.window.backingScaleFactor;
    self.layer.drawableSize = CGSizeMake(600 * self.layer.contentsScale, 400 * self.layer.contentsScale);
    self.window.contentView.wantsLayer = YES;
    self.window.contentView.layer = self.layer;
    [self.window makeKeyAndOrderFront:nil];
    [NSApp activate];
    [CATransaction flush];
    NSInteger hz = self.window.screen.maximumFramesPerSecond;
    printf("window source=%s visible=%d screen=%s max_hz=%ld layer=%p framebuffer=%.0fx%.0f\n",
           self.mode.UTF8String, self.window.visible, self.window.screen.localizedName.UTF8String,
           (long)hz, self.layer, self.layer.drawableSize.width, self.layer.drawableSize.height);

    if ([self.mode isEqualToString:@"next"]) {
        NSTimer *timer = [NSTimer timerWithTimeInterval:1.0 / (hz > 0 ? hz : 60)
                                              repeats:YES block:^(NSTimer *tick) {
            @autoreleasepool { [self encode:[self.layer nextDrawable] source:"next"]; }
        }];
        [NSRunLoop.mainRunLoop addTimer:timer forMode:NSRunLoopCommonModes];
    } else {
        if ([self.mode isEqualToString:@"hybrid"])
            [self encode:[self.layer nextDrawable] source:"next"];
        [self attachLink];
        if ([self.mode isEqualToString:@"hybrid"]) {
            NSTimer *timer = [NSTimer timerWithTimeInterval:2.0 repeats:YES block:^(NSTimer *tick) {
                @autoreleasepool {
                    [self.link invalidate];
                    self.link = nil;
                    [self encode:[self.layer nextDrawable] source:"next"];
                    [self attachLink];
                }
            }];
            [NSRunLoop.mainRunLoop addTimer:timer forMode:NSRunLoopCommonModes];
        }
    }
    [NSTimer scheduledTimerWithTimeInterval:15.0 repeats:NO block:^(NSTimer *tick) {
        [self.link invalidate];
        [NSApp terminate:nil];
    }];
}

@end

int main(int argc, char **argv) {
    if (argc != 4) return 2;
    setvbuf(stdout, NULL, _IOLBF, 0);
    @autoreleasepool {
        [NSApplication sharedApplication];
        [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];
        __attribute__((objc_precise_lifetime)) DrawableProbe *probe = [DrawableProbe new];
        probe.mode = [NSString stringWithUTF8String:argv[1]];
        probe.limit = (NSUInteger)strtoul(argv[2], NULL, 10);
        probe.latency = (float)atof(argv[3]);
        NSApp.delegate = probe;
        [NSApp run];
    }
    return 0;
}
