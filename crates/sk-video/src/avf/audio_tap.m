#import <AVFoundation/AVFoundation.h>
#import <AudioToolbox/AudioToolbox.h>
#import <MediaToolbox/MediaToolbox.h>
#include <stdbool.h>
#include <stdlib.h>
#include <string.h>

typedef void (*Push)(void *, const float *, size_t, uint32_t, uint32_t);
typedef void (*Release)(void *);

@interface SKMediaAudioTap : NSObject {
@public
    void *context;
    Push push;
    Release releaseContext;
    AudioConverterRef converter;
    AudioBufferList output;
    size_t capacity;
    uint32_t rate;
}
@end

@implementation SKMediaAudioTap
- (void)dealloc {
    if (converter) AudioConverterDispose(converter);
    free(output.mBuffers[0].mData);
    releaseContext(context);
}
@end

static void initialize(MTAudioProcessingTapRef tap, void *info, void **storage) {
    *storage = (__bridge_retained void *)(__bridge SKMediaAudioTap *)info;
}

static void finalize(MTAudioProcessingTapRef tap) {
    CFRelease(MTAudioProcessingTapGetStorage(tap));
}

static void unprepare(MTAudioProcessingTapRef tap) {
    SKMediaAudioTap *state = (__bridge SKMediaAudioTap *)MTAudioProcessingTapGetStorage(tap);
    if (state->converter) AudioConverterDispose(state->converter);
    state->converter = NULL;
    free(state->output.mBuffers[0].mData);
    state->output.mBuffers[0].mData = NULL;
    state->capacity = 0;
}

static void prepare(MTAudioProcessingTapRef tap, CMItemCount maxFrames, const AudioStreamBasicDescription *input) {
    SKMediaAudioTap *state = (__bridge SKMediaAudioTap *)MTAudioProcessingTapGetStorage(tap);
    UInt32 channels = input->mChannelsPerFrame;
    if (channels == 0 || channels > 32 || maxFrames <= 0 || (size_t)maxFrames > UINT32_MAX / (channels * sizeof(float))) return;
    AudioStreamBasicDescription format = {
        .mFormatID = kAudioFormatLinearPCM, .mFormatFlags = kAudioFormatFlagsNativeFloatPacked,
        .mSampleRate = input->mSampleRate, .mChannelsPerFrame = channels,
        .mBytesPerFrame = channels * sizeof(float), .mBytesPerPacket = channels * sizeof(float),
        .mFramesPerPacket = 1, .mBitsPerChannel = 32,
    };
    if (AudioConverterNew(input, &format, &state->converter) != noErr) return;
    state->rate = (uint32_t)format.mSampleRate;
    state->output.mNumberBuffers = 1;
    state->output.mBuffers[0].mNumberChannels = channels;
    state->capacity = maxFrames * format.mBytesPerFrame;
    state->output.mBuffers[0].mData = malloc(state->capacity);
}

static void process(MTAudioProcessingTapRef tap, CMItemCount count, MTAudioProcessingTapFlags flags,
                    AudioBufferList *audio, CMItemCount *produced, MTAudioProcessingTapFlags *outFlags) {
    *produced = 0;
    if (MTAudioProcessingTapGetSourceAudio(tap, count, audio, outFlags, NULL, produced) != noErr) return;
    SKMediaAudioTap *state = (__bridge SKMediaAudioTap *)MTAudioProcessingTapGetStorage(tap);
    AudioBuffer *out = &state->output.mBuffers[0];
    out->mDataByteSize = (UInt32)state->capacity;
    if (*produced > 0 && state->converter && out->mData &&
        AudioConverterConvertComplexBuffer(state->converter, (UInt32)*produced, audio, &state->output) == noErr) {
        state->push(state->context, out->mData, out->mDataByteSize / sizeof(float), out->mNumberChannels, state->rate);
    }
    for (UInt32 i = 0; i < audio->mNumberBuffers; ++i) {
        AudioBuffer *buffer = &audio->mBuffers[i];
        if (buffer->mData) memset(buffer->mData, 0, buffer->mDataByteSize);
    }
}

// Route the first enabled audio track and silence the player's other tracks for this mix.
bool sk_install_audio_tap(AVPlayerItem *item, void *context, Push push, Release releaseContext) {
    SKMediaAudioTap *owner __attribute__((objc_precise_lifetime)) = [SKMediaAudioTap new];
    owner->context = context;
    owner->push = push;
    owner->releaseContext = releaseContext;
    NSMutableArray *params = [NSMutableArray array];
    for (AVPlayerItemTrack *itemTrack in item.tracks) {
        AVAssetTrack *track = itemTrack.assetTrack;
        if (!itemTrack.enabled || ![track.mediaType isEqual:AVMediaTypeAudio]) continue;
        AVMutableAudioMixInputParameters *input = [AVMutableAudioMixInputParameters audioMixInputParametersWithTrack:track];
        if (params.count > 0) {
            [input setVolume:0 atTime:kCMTimeZero];
            [params addObject:input];
            continue;
        }
        MTAudioProcessingTapCallbacks callbacks = {
            kMTAudioProcessingTapCallbacksVersion_0, (__bridge void *)owner, initialize, finalize, prepare, unprepare, process,
        };
        MTAudioProcessingTapRef tap;
        if (MTAudioProcessingTapCreate(kCFAllocatorDefault, &callbacks,
                                       kMTAudioProcessingTapCreationFlag_PostEffects, &tap) != noErr) return false;
        input.audioTapProcessor = tap;
        CFRelease(tap);
        [params addObject:input];
    }
    if (params.count == 0) return false;
    AVMutableAudioMix *mix = [AVMutableAudioMix audioMix];
    mix.inputParameters = params;
    item.audioMix = mix;
    return true;
}
