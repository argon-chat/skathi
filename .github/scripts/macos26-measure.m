#import <AppKit/AppKit.h>
#include <inttypes.h>
#include <libproc.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <time.h>
#include <unistd.h>

static double monotonic_ms(void) {
    struct timespec at;
    clock_gettime(CLOCK_MONOTONIC, &at);
    return at.tv_sec * 1000.0 + at.tv_nsec / 1000000.0;
}

static int sample(pid_t pid) {
    puts("elapsed_ms\tphysical_footprint_bytes\tlifetime_peak_footprint_bytes");
    double start = monotonic_ms();
    unsigned count = 0;
    for (;;) {
        double elapsed = monotonic_ms() - start;
        if (elapsed > 13500.0) break;
        struct rusage_info_v4 usage = {0};
        if (proc_pid_rusage(pid, RUSAGE_INFO_V4, (rusage_info_t *)&usage) < 0) {
            perror("proc_pid_rusage");
            break;
        }
        printf("%.2f\t%" PRIu64 "\t%" PRIu64 "\n", elapsed,
               (uint64_t)usage.ri_phys_footprint, (uint64_t)usage.ri_lifetime_max_phys_footprint);
        count++;
        usleep(20000);
    }
    return count == 0;
}

static int list_windows(pid_t pid) {
    CFArrayRef entries = CGWindowListCopyWindowInfo(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements, kCGNullWindowID);
    NSArray<NSDictionary *> *windows = CFBridgingRelease(entries);
    unsigned count = 0;
    for (NSDictionary *window in windows) {
        NSNumber *owner = window[(__bridge NSString *)kCGWindowOwnerPID];
        NSNumber *layer = window[(__bridge NSString *)kCGWindowLayer];
        if (owner.intValue != pid || layer.intValue != 0) continue;
        CGRect bounds;
        if (!CGRectMakeWithDictionaryRepresentation(
            (__bridge CFDictionaryRef)window[(__bridge NSString *)kCGWindowBounds], &bounds)) continue;
        if (bounds.size.width <= 0 || bounds.size.height <= 0) continue;
        printf("on-screen window %.0fx%.0f at %.0f,%.0f\n",
               bounds.size.width, bounds.size.height, bounds.origin.x, bounds.origin.y);
        count++;
    }
    if (!count) fprintf(stderr, "No on-screen window for PID %d\n", pid);
    return count == 0;
}

static int list_displays(void) {
    NSArray<NSScreen *> *screens = NSScreen.screens;
    printf("AppKit screens: %lu\n", (unsigned long)screens.count);
    for (NSScreen *screen in screens) {
        printf("%s %.0fx%.0f pt, scale %.1f, maximum %ld Hz, current EDR %.2f, potential EDR %.2f\n",
               screen.localizedName.UTF8String, screen.frame.size.width, screen.frame.size.height,
               screen.backingScaleFactor, (long)screen.maximumFramesPerSecond,
               screen.maximumExtendedDynamicRangeColorComponentValue,
               screen.maximumPotentialExtendedDynamicRangeColorComponentValue);
    }
    return 0;
}

int main(int argc, char **argv) {
    @autoreleasepool {
        if (argc == 2 && strcmp(argv[1], "displays") == 0) return list_displays();
        if (argc == 3) {
            pid_t pid = (pid_t)atoi(argv[2]);
            if (pid <= 1) return 2;
            if (strcmp(argv[1], "sample") == 0) return sample(pid);
            if (strcmp(argv[1], "window") == 0) return list_windows(pid);
        }
        return 2;
    }
}
