RZ_METAL_SOURCE(
using namespace metal;

float matchError(texture2d<half, access::read> previous,
                 texture2d<half, access::read> current,
                 int2 origin, int2 offset, int2 limit) {
    float error = 0;
    for (int y = 1; y < 8; y += 3) {
        for (int x = 1; x < 8; x += 3) {
            int2 point = origin + int2(x, y);
            half3 a = current.read(uint2(min(point, limit))).rgb;
            half3 b = previous.read(uint2(clamp(point + offset, int2(0), limit))).rgb;
            error += float(dot(abs(a - b), half3(0.3, 0.59, 0.11)));
        }
    }
    return error;
}

// Search one motion vector per source block. Ties prefer stationary pixels.
kernel void findMotion(texture2d<half, access::read> previous [[texture(0)]],
                       texture2d<half, access::read> current [[texture(1)]],
                       texture2d<half, access::write> blocks [[texture(2)]],
                       uint2 block [[thread_position_in_grid]]) {
    int2 origin = int2(block * 8);
    int2 limit = int2(current.get_width() - 1, current.get_height() - 1);
    int2 bestOffset = int2(0);
    float best = matchError(previous, current, origin, bestOffset, limit);
    if (best > 0.025f) {
        for (int y = -4; y <= 4; y += 2) {
            for (int x = -4; x <= 4; x += 2) {
                int2 offset = int2(x, y);
                float error = matchError(previous, current, origin, offset, limit)
                              + 0.025f * float(abs(x) + abs(y));
                if (error < best) {
                    best = error;
                    bestOffset = offset;
                }
            }
        }
        int2 coarse = bestOffset;
        for (int y = -1; y <= 1; ++y) {
            for (int x = -1; x <= 1; ++x) {
                int2 offset = coarse + int2(x, y);
                float error = matchError(previous, current, origin, offset, limit)
                              + 0.025f * float(abs(offset.x) + abs(offset.y));
                if (error < best) {
                    best = error;
                    bestOffset = offset;
                }
            }
        }
    }
    blocks.write(half4(half2(bestOffset), 0, 0), block);
}

kernel void fillMotion(texture2d<half, access::read> previous [[texture(0)]],
                       texture2d<half, access::read> current [[texture(1)]],
                       texture2d<half, access::read> blocks [[texture(2)]],
                       texture2d<half, access::write> motion [[texture(3)]],
                       texture2d<float, access::write> depth [[texture(4)]],
                       uint2 pixel [[thread_position_in_grid]]) {
    // The 2D framebuffer has no camera depth. Use one plane and keep unchanged pixels stationary.
    const half3 difference = abs(current.read(pixel).rgb - previous.read(pixel).rgb);
    motion.write(any(difference > half3(0.01)) ? blocks.read(pixel / 8) : half4(0), pixel);
    depth.write(float4(0.5f), pixel);
}
)
