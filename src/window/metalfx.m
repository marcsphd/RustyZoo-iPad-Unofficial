#import <MetalFX/MetalFX.h>
#import <QuartzCore/CAMetalLayer.h>
#include <stdatomic.h>
#include <stdbool.h>
#include <stdint.h>
#include <string.h>

#if __MAC_OS_X_VERSION_MAX_ALLOWED >= 260000
#define RZ_FRAME_GENERATION 1
#define RZ_METAL_SOURCE(...) @#__VA_ARGS__
static NSString *const motionSource = @"#include <metal_stdlib>\n"
#include "metalfx_motion.inc"
;
#undef RZ_METAL_SOURCE

API_AVAILABLE(macos(26.0))
@interface RustyZooInterpolation : NSObject
@property(nonatomic, strong) id<MTLFXFrameInterpolator> interpolator;
@property(nonatomic, strong) id<MTLComputePipelineState> findMotion;
@property(nonatomic, strong) id<MTLComputePipelineState> fillMotion;
@property(nonatomic, strong) id<MTLTexture> previousInput;
@property(nonatomic, strong) id<MTLTexture> previousOutput;
@property(nonatomic, strong) id<MTLTexture> blocks;
@property(nonatomic, strong) id<MTLTexture> motion;
@property(nonatomic, strong) id<MTLTexture> depth;
@property(nonatomic, strong) id<MTLTexture> generated;
@property(nonatomic) CFTimeInterval previousTime;
@property(nonatomic) BOOL reset;
@property(nonatomic) BOOL failed;
@end

@implementation RustyZooInterpolation
@end
#endif

API_AVAILABLE(macos(13.0))
@interface RustyZooScaler : NSObject
@property(nonatomic, strong) CAMetalLayer *layer;
@property(nonatomic, strong) id<MTLCommandQueue> queue;
@property(nonatomic, strong) id<MTLFXSpatialScaler> scaler;
@property(nonatomic, strong) id<MTLTexture> input;
@property(nonatomic, strong) id<MTLTexture> output;
@property(nonatomic, strong) id<MTLCommandBuffer> lastFrame;
@property(nonatomic) BOOL lastFrameGenerated;
#if RZ_FRAME_GENERATION
@property(nonatomic, strong) RustyZooInterpolation *generation API_AVAILABLE(macos(26.0));
#endif
@end

@implementation RustyZooScaler
@end

void *rustyzoo_metalfx_create(void *metalLayer, bool frameGeneration) {
    @autoreleasepool {
        if (@available(macOS 13.0, *)) {
            CAMetalLayer *layer = (__bridge CAMetalLayer *)metalLayer;
            if (!layer || ![MTLFXSpatialScalerDescriptor supportsDevice:layer.device] ||
                layer.pixelFormat != MTLPixelFormatBGRA8Unorm || layer.framebufferOnly) {
                return NULL;
            }
            RustyZooScaler *fx = [RustyZooScaler new];
            fx.layer = layer;
            fx.queue = [layer.device newCommandQueue];
            if (frameGeneration) {
#if RZ_FRAME_GENERATION
                if (@available(macOS 26.0, *)) {
                    if ([MTLFXFrameInterpolatorDescriptor supportsDevice:layer.device]) {
                        fx.generation = [RustyZooInterpolation new];
                    }
                }
                if (@available(macOS 26.0, *)) {
                    if (!fx.generation) {
                        fprintf(stderr, "rustyzoo: MetalFX frame generation is unavailable on this GPU\n");
                    }
                } else
#endif
                {
                    fprintf(stderr, "rustyzoo: MetalFX frame generation requires macOS 26 and its SDK\n");
                }
            }
            return fx.queue ? (__bridge_retained void *)fx : NULL;
        }
        return NULL;
    }
}

void rustyzoo_metalfx_destroy(void *context) {
    @autoreleasepool {
        (void)CFBridgingRelease(context);
    }
}

#if RZ_FRAME_GENERATION
API_AVAILABLE(macos(26.0))
static id<MTLTexture> privateTexture(id<MTLDevice> device, MTLPixelFormat format,
                                     NSUInteger w, NSUInteger h, MTLTextureUsage usage) {
    MTLTextureDescriptor *desc = [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:format
                                                                                    width:w height:h
                                                                                mipmapped:NO];
    desc.storageMode = MTLStorageModePrivate;
    desc.usage = usage;
    return [device newTextureWithDescriptor:desc];
}

API_AVAILABLE(macos(26.0))
static void configureInterpolation(RustyZooScaler *fx, MTLTextureDescriptor *output,
                                   NSUInteger w, NSUInteger h) {
    RustyZooInterpolation *gen = fx.generation;
    if (!gen || gen.failed) {
        return;
    }
    id<MTLDevice> device = fx.layer.device;
    if (!gen.findMotion || !gen.fillMotion) {
        NSError *error = nil;
        id<MTLLibrary> library = [device newLibraryWithSource:motionSource options:nil error:&error];
        if (library) {
            gen.findMotion = [device newComputePipelineStateWithFunction:[library newFunctionWithName:@"findMotion"]
                                                                    error:&error];
            gen.fillMotion = [device newComputePipelineStateWithFunction:[library newFunctionWithName:@"fillMotion"]
                                                                    error:&error];
        }
        if (!gen.findMotion || !gen.fillMotion) {
            fprintf(stderr, "rustyzoo: MetalFX motion shader failed: %s\n", error.localizedDescription.UTF8String ?: "unknown error");
            gen.failed = YES;
            return;
        }
    }
    MTLFXFrameInterpolatorDescriptor *desc = [MTLFXFrameInterpolatorDescriptor new];
    desc.inputWidth = w;
    desc.inputHeight = h;
    desc.outputWidth = output.width;
    desc.outputHeight = output.height;
    desc.colorTextureFormat = output.pixelFormat;
    desc.outputTextureFormat = output.pixelFormat;
    desc.motionTextureFormat = MTLPixelFormatRG16Float;
    desc.depthTextureFormat = MTLPixelFormatR32Float;

    id<MTLFXFrameInterpolator> effect = [desc newFrameInterpolatorWithDevice:device];
    gen.previousInput = privateTexture(device, MTLPixelFormatRGBA8Unorm, w, h, MTLTextureUsageShaderRead);
    gen.blocks = privateTexture(device, MTLPixelFormatRG16Float, (w + 7) / 8, (h + 7) / 8,
                                MTLTextureUsageShaderRead | MTLTextureUsageShaderWrite);
    gen.motion = privateTexture(device, desc.motionTextureFormat, w, h,
                                effect.motionTextureUsage | MTLTextureUsageShaderWrite);
    gen.depth = privateTexture(device, desc.depthTextureFormat, w, h,
                               effect.depthTextureUsage | MTLTextureUsageShaderWrite);
    gen.generated = privateTexture(device, desc.outputTextureFormat, output.width, output.height,
                                   effect.outputTextureUsage);
    output.usage |= effect.colorTextureUsage;
    gen.previousOutput = [device newTextureWithDescriptor:output];
    gen.interpolator = effect;
    gen.previousTime = 0;
    gen.reset = YES;
    if (!effect || !gen.previousInput || !gen.blocks || !gen.motion || !gen.depth ||
        !gen.generated || !gen.previousOutput) {
        fprintf(stderr, "rustyzoo: MetalFX frame generation could not configure its textures\n");
        gen.failed = YES;
        gen.interpolator = nil;
        return;
    }
    effect.motionVectorScaleX = 1;
    effect.motionVectorScaleY = 1;
    effect.depthReversed = NO;
    effect.nearPlane = 0.1f;
    effect.farPlane = 100.0f;
    effect.fieldOfView = 60.0f;
    effect.aspectRatio = (float)output.width / (float)output.height;
    effect.motionTexture = gen.motion;
    effect.depthTexture = gen.depth;
    effect.outputTexture = gen.generated;
}

API_AVAILABLE(macos(26.0))
static BOOL encodeInterpolation(RustyZooScaler *fx, id<MTLCommandBuffer> commands,
                                CFTimeInterval interval) {
    RustyZooInterpolation *gen = fx.generation;
    id<MTLComputeCommandEncoder> search = [commands computeCommandEncoder];
    if (!search) {
        return NO;
    }
    [search setComputePipelineState:gen.findMotion];
    [search setTexture:gen.previousInput atIndex:0];
    [search setTexture:fx.input atIndex:1];
    [search setTexture:gen.blocks atIndex:2];
    [search dispatchThreads:MTLSizeMake(gen.blocks.width, gen.blocks.height, 1)
      threadsPerThreadgroup:MTLSizeMake(8, 8, 1)];
    [search endEncoding];

    id<MTLComputeCommandEncoder> fill = [commands computeCommandEncoder];
    if (!fill) {
        return NO;
    }
    [fill setComputePipelineState:gen.fillMotion];
    [fill setTexture:gen.previousInput atIndex:0];
    [fill setTexture:fx.input atIndex:1];
    [fill setTexture:gen.blocks atIndex:2];
    [fill setTexture:gen.motion atIndex:3];
    [fill setTexture:gen.depth atIndex:4];
    [fill dispatchThreads:MTLSizeMake(fx.input.width, fx.input.height, 1)
    threadsPerThreadgroup:MTLSizeMake(8, 8, 1)];
    [fill endEncoding];

    id<MTLFXFrameInterpolator> effect = gen.interpolator;
    effect.colorTexture = fx.output;
    effect.prevColorTexture = gen.previousOutput;
    effect.deltaTime = interval;
    effect.shouldResetHistory = gen.reset;
    [effect encodeToCommandBuffer:commands];
    gen.reset = NO;
    return YES;
}
#endif

API_AVAILABLE(macos(13.0))
static BOOL configure(RustyZooScaler *fx, uint32_t w, uint32_t h, uint32_t outW, uint32_t outH) {
    id<MTLDevice> device = fx.layer.device;
    MTLFXSpatialScalerDescriptor *desc = [MTLFXSpatialScalerDescriptor new];
    desc.inputWidth = w;
    desc.inputHeight = h;
    desc.outputWidth = outW;
    desc.outputHeight = outH;
    desc.colorTextureFormat = MTLPixelFormatRGBA8Unorm;
    desc.outputTextureFormat = fx.layer.pixelFormat;
    desc.colorProcessingMode = MTLFXSpatialScalerColorProcessingModePerceptual;

    id<MTLFXSpatialScaler> scaler = [desc newSpatialScalerWithDevice:device];
    if (!scaler) {
        return NO;
    }
    MTLTextureDescriptor *texture = [MTLTextureDescriptor
        texture2DDescriptorWithPixelFormat:desc.colorTextureFormat width:w height:h mipmapped:NO];
    // Use a CPU-writable texture for the software framebuffer.
#if defined(__arm64__)
    texture.storageMode = MTLStorageModeShared;
#else
    texture.storageMode = MTLStorageModeManaged;
#endif
    texture.usage = scaler.colorTextureUsage | MTLTextureUsageShaderRead;
    id<MTLTexture> input = [device newTextureWithDescriptor:texture];

    texture.pixelFormat = desc.outputTextureFormat;
    texture.width = outW;
    texture.height = outH;
    texture.storageMode = MTLStorageModePrivate;
    texture.usage = scaler.outputTextureUsage;
#if RZ_FRAME_GENERATION
    if (@available(macOS 26.0, *)) {
        configureInterpolation(fx, texture, w, h);
    }
#endif
    id<MTLTexture> output = [device newTextureWithDescriptor:texture];
    if (!input || !output) {
        return NO;
    }

    scaler.inputContentWidth = w;
    scaler.inputContentHeight = h;
    scaler.colorTexture = input;
    scaler.outputTexture = output;
    fx.input = input;
    fx.output = output;
    fx.scaler = scaler;
    return YES;
}

API_AVAILABLE(macos(13.0))
static BOOL clearBorders(id<MTLCommandBuffer> commands, id<MTLTexture> texture,
                         uint32_t width, uint32_t height) {
    if (width == texture.width && height == texture.height) {
        return YES;
    }
    MTLRenderPassDescriptor *pass = [MTLRenderPassDescriptor renderPassDescriptor];
    pass.colorAttachments[0].texture = texture;
    pass.colorAttachments[0].loadAction = MTLLoadActionClear;
    pass.colorAttachments[0].storeAction = MTLStoreActionStore;
    pass.colorAttachments[0].clearColor = MTLClearColorMake(0, 0, 0, 1);
    id<MTLRenderCommandEncoder> clear = [commands renderCommandEncoderWithDescriptor:pass];
    [clear endEncoding];
    return clear != nil;
}

// The drawable is borrowed from SDL's Metal layer. SDL performs no rendering during this pass.
// Returns 0 for failure, 1 for a real frame, 2 for no drawable, or 3 with an interpolated frame.
int rustyzoo_metalfx_present(void *context, const uint8_t *pixels, uint32_t w, uint32_t h,
                             uint32_t x, uint32_t y, uint32_t outW, uint32_t outH,
                             uint8_t *capture, uint32_t captureW, uint32_t captureH, bool interpolate) {
    @autoreleasepool {
        if (@available(macOS 13.0, *)) {
            RustyZooScaler *fx = (__bridge RustyZooScaler *)context;
            [fx.lastFrame waitUntilCompleted];
            if (fx.lastFrame.status == MTLCommandBufferStatusError) {
                fprintf(stderr, "rustyzoo: Metal command buffer failed: %s\n",
                        fx.lastFrame.error.localizedDescription.UTF8String ?: "unknown error");
#if RZ_FRAME_GENERATION
                if (@available(macOS 26.0, *)) {
                    if (fx.lastFrameGenerated) {
                        fx.generation = nil;
                        fx.lastFrame = nil;
                        fprintf(stderr, "rustyzoo: continuing with MetalFX spatial scaling\n");
                    }
                }
#endif
                if (fx.lastFrame) {
                    return 0;
                }
            }
            if (!fx.scaler || fx.input.width != w || fx.input.height != h ||
                fx.output.width != outW || fx.output.height != outH) {
                if (!configure(fx, w, h, outW, outH)) {
                    return 0;
                }
            }

            id<CAMetalDrawable> drawable = [fx.layer nextDrawable];
            if (!drawable || x + outW > drawable.texture.width || y + outH > drawable.texture.height ||
                (capture && (captureW != drawable.texture.width || captureH != drawable.texture.height))) {
                return 2;
            }

            [fx.input replaceRegion:MTLRegionMake2D(0, 0, w, h) mipmapLevel:0
                          withBytes:pixels bytesPerRow:(NSUInteger)w * 4];
            id<MTLCommandBuffer> commands = [fx.queue commandBuffer];
            if (!commands) {
                return 0;
            }
            [fx.scaler encodeToCommandBuffer:commands];

            id<CAMetalDrawable> intermediate = nil;
            id<MTLTexture> generated = nil;
            CFTimeInterval interval = 0;
#if RZ_FRAME_GENERATION
            if (@available(macOS 26.0, *)) {
                RustyZooInterpolation *gen = fx.generation;
                if (gen.interpolator) {
                    CFTimeInterval now = CACurrentMediaTime();
                    interval = now - gen.previousTime;
                    if (interpolate && !capture && gen.previousTime != 0 &&
                        interval >= 0.012 && interval <= 0.075) {
                        intermediate = [fx.layer nextDrawable];
                        if (intermediate.texture.width == drawable.texture.width &&
                            intermediate.texture.height == drawable.texture.height &&
                            encodeInterpolation(fx, commands, interval)) {
                            generated = gen.generated;
                        } else {
                            intermediate = nil;
                        }
                    }
                    gen.previousTime = interpolate ? now : 0;
                    if (!generated) {
                        gen.reset = YES;
                    }
                }
            }
#else
            (void)interpolate;
#endif
            // Present in acquisition order and use one presentation command buffer per drawable.
            id<MTLCommandBuffer> realPresentation = nil;
            if (intermediate) {
                realPresentation = [fx.queue commandBuffer];
                if (realPresentation) {
                    id<CAMetalDrawable> first = drawable;
                    drawable = intermediate;
                    intermediate = first;
                } else {
                    generated = nil;
                    intermediate = nil;
                }
            }
            if (!clearBorders(commands, drawable.texture, outW, outH) ||
                (intermediate && !clearBorders(commands, intermediate.texture, outW, outH))) {
                return 0;
            }

            const NSUInteger pitch = ((NSUInteger)captureW * 4 + 255) & ~(NSUInteger)255;
            id<MTLBuffer> readback = nil;
            if (capture) {
                readback = [fx.layer.device newBufferWithLength:pitch * captureH
                                                       options:MTLResourceStorageModeShared];
                if (!readback) {
                    return 0;
                }
            }
            id<MTLBlitCommandEncoder> blit = [commands blitCommandEncoder];
            if (!blit) {
                return 0;
            }
            [blit copyFromTexture:fx.output sourceSlice:0 sourceLevel:0
                     sourceOrigin:MTLOriginMake(0, 0, 0) sourceSize:MTLSizeMake(outW, outH, 1)
                        toTexture:drawable.texture destinationSlice:0 destinationLevel:0
                destinationOrigin:MTLOriginMake(x, y, 0)];
            if (generated) {
                [blit copyFromTexture:generated sourceSlice:0 sourceLevel:0
                         sourceOrigin:MTLOriginMake(0, 0, 0) sourceSize:MTLSizeMake(outW, outH, 1)
                            toTexture:intermediate.texture destinationSlice:0 destinationLevel:0
                    destinationOrigin:MTLOriginMake(x, y, 0)];
            }
#if RZ_FRAME_GENERATION
            if (@available(macOS 26.0, *)) {
                RustyZooInterpolation *gen = fx.generation;
                if (gen.interpolator && interpolate) {
                    [blit copyFromTexture:fx.input toTexture:gen.previousInput];
                    [blit copyFromTexture:fx.output toTexture:gen.previousOutput];
                }
            }
#endif
            if (capture) {
                [blit copyFromTexture:drawable.texture sourceSlice:0 sourceLevel:0
                         sourceOrigin:MTLOriginMake(0, 0, 0) sourceSize:MTLSizeMake(captureW, captureH, 1)
                             toBuffer:readback destinationOffset:0 destinationBytesPerRow:pitch
                    destinationBytesPerImage:0];
            }
            [blit endEncoding];
            if (intermediate) {
                static atomic_bool verified = false;
                if (!atomic_load(&verified)) {
                    [intermediate addPresentedHandler:^(id<MTLDrawable> shown) {
                        if (shown.presentedTime > 0 && !atomic_exchange(&verified, true)) {
                            fprintf(stderr, "MetalFX interpolated frame displayed\n");
                        }
                    }];
                }
                [commands presentDrawable:intermediate];
                [realPresentation presentDrawable:drawable afterMinimumDuration:interval * 0.5];
            } else {
                [commands presentDrawable:drawable];
            }
            [commands commit];
            [realPresentation commit];
            fx.lastFrame = commands;
            fx.lastFrameGenerated = intermediate != nil;

            if (capture) {
                [commands waitUntilCompleted];
                if (commands.status != MTLCommandBufferStatusCompleted) {
                    return 0;
                }
                const uint8_t *bytes = readback.contents;
                for (uint32_t row = 0; row < captureH; row++) {
                    memcpy(capture + (NSUInteger)row * captureW * 4, bytes + row * pitch,
                           (NSUInteger)captureW * 4);
                }
            }
            return intermediate ? 3 : 1;
        }
        return 0;
    }
}
