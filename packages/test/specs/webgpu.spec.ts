import { afterAll, expect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/events/src/main.ts",
  window: [480, 320],
  label: "webgpu",
});
afterAll(async () => {
  await app.close();
});

const T = 60_000;

async function inPage(body: string): Promise<unknown> {
  await app.eval(
    `globalThis.__gpu = undefined; (async () => { ${body} })().then(` +
      `v => { globalThis.__gpu = JSON.stringify({ ok: v }); }, ` +
      `e => { globalThis.__gpu = JSON.stringify({ err: (e && e.name) + ': ' + (e && e.message) }); }); 'started'`,
  );
  const until = Date.now() + 20_000;
  while (Date.now() < until) {
    const raw = await app.eval("globalThis.__gpu === undefined ? '' : globalThis.__gpu");
    if (raw) return JSON.parse(String(raw));
    await Bun.sleep(50);
  }
  throw new Error("the page body never settled");
}

test(
  "a canvas draws through a render bundle and indirect draws, with occlusion and timestamp queries",
  async () => {
    const got = await inPage(`
    const adapter = await navigator.gpu.requestAdapter();
    const timestamps = adapter.features.has('timestamp-query');
    const d = await adapter.requestDevice({ requiredFeatures: timestamps ? ['timestamp-query'] : [] });
    const canvas = document.createElement('canvas');
    canvas.width = 8; canvas.height = 8;
    const context = canvas.getContext('webgpu');
    context.configure({ device: d, format: 'rgba8unorm', alphaMode: 'opaque',
      usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.COPY_SRC });
    const module = d.createShaderModule({ code: \`
      struct V { @builtin(position) p: vec4f, @location(0) c: vec4f }
      @vertex fn vs(@location(0) p: vec2f, @location(1) c: vec4f) -> V { return V(vec4f(p, 0, 1), c); }
      @fragment fn fs(v: V) -> @location(0) vec4f { return v.c; }\` });
    const pipeline = await d.createRenderPipelineAsync({ layout: 'auto',
      vertex: { module, buffers: [
        { arrayStride: 8, attributes: [{ shaderLocation: 0, offset: 0, format: 'float32x2' }] },
        { arrayStride: 4, attributes: [{ shaderLocation: 1, offset: 0, format: 'unorm8x4' }] }] },
      fragment: { module, targets: [{ format: 'rgba8unorm' }] } });
    const buffer = (usage, data) => {
      const b = d.createBuffer({ size: data.byteLength, usage: usage | GPUBufferUsage.COPY_DST });
      d.queue.writeBuffer(b, 0, data); return b;
    };
    const RED = [255, 0, 0, 255], GREEN = [0, 255, 0, 255];
    const positions = buffer(GPUBufferUsage.VERTEX, new Float32Array([-1,-1, 0,-1, -1,1, 0,-1, 0,1, -1,1, 0,-1, 1,-1, 0,1]));
    const colors = buffer(GPUBufferUsage.VERTEX, new Uint8Array([...Array(6).fill(RED).flat(), ...Array(3).fill(GREEN).flat()]));
    const indices = buffer(GPUBufferUsage.INDEX, new Uint16Array([0, 1, 2, 0]));
    const args = new Uint32Array(10); args.set([6, 1, 0, 0], 0); args.set([3, 1, 0, 6, 0], 4);
    const indirect = buffer(GPUBufferUsage.INDIRECT, args);

    const recorder = d.createRenderBundleEncoder({ colorFormats: ['rgba8unorm'], label: 'left half' });
    recorder.setPipeline(pipeline); recorder.setVertexBuffer(0, positions); recorder.setVertexBuffer(1, colors);
    recorder.drawIndirect(indirect, 0);
    const bundle = recorder.finish();

    const occlusion = d.createQuerySet({ type: 'occlusion', count: 2 });
    const times = timestamps ? d.createQuerySet({ type: 'timestamp', count: 2 }) : undefined;
    const target = context.getCurrentTexture();
    const encoder = d.createCommandEncoder();
    encoder.pushDebugGroup('frame');
    const pass = encoder.beginRenderPass({
      colorAttachments: [{ view: target.createView(), loadOp: 'clear', storeOp: 'store', clearValue: [0, 0, 1, 1] }],
      occlusionQuerySet: occlusion,
      ...(times ? { timestampWrites: { querySet: times, beginningOfPassWriteIndex: 0, endOfPassWriteIndex: 1 } } : {}),
    });
    pass.executeBundles([bundle]);
    pass.setPipeline(pipeline); pass.setVertexBuffer(0, positions); pass.setVertexBuffer(1, colors);
    pass.setIndexBuffer(indices, 'uint16');
    pass.beginOcclusionQuery(0); pass.drawIndexedIndirect(indirect, 16); pass.endOcclusionQuery();
    pass.beginOcclusionQuery(1); pass.endOcclusionQuery();
    pass.end();
    encoder.popDebugGroup();
    const pixels = d.createBuffer({ size: 256 * 8, usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ });
    encoder.copyTextureToBuffer({ texture: target }, { buffer: pixels, bytesPerRow: 256 }, [8, 8]);
    const queries = (set, count) => {
      const resolved = d.createBuffer({ size: 8 * count, usage: GPUBufferUsage.QUERY_RESOLVE | GPUBufferUsage.COPY_SRC });
      const read = d.createBuffer({ size: 8 * count, usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ });
      encoder.resolveQuerySet(set, 0, count, resolved, 0);
      encoder.copyBufferToBuffer(resolved, 0, read, 0, 8 * count);
      return read;
    };
    const occluded = queries(occlusion, 2);
    const timed = times && queries(times, 2);
    d.pushErrorScope('validation');
    d.queue.submit([encoder.finish()]);
    const error = await d.popErrorScope();
    await Promise.all([pixels, occluded, timed].filter(Boolean).map(b => b.mapAsync(GPUMapMode.READ)));
    const rgba = new Uint8Array(pixels.getMappedRange());
    const pixel = (x, y) => Array.from(rgba.slice(y * 256 + x * 4, y * 256 + x * 4 + 4)).join();
    const [drawn, empty] = new BigUint64Array(occluded.getMappedRange());
    const [begin, end] = timed ? new BigUint64Array(timed.getMappedRange()) : [0n, 0n];
    return {
      error: error && error.message,
      left: pixel(1, 3), triangle: pixel(4, 7), clear: pixel(7, 0),
      label: bundle.label === '' && recorder.label === 'left half',
      occlusion: drawn > 0n && empty === 0n,
      timestamps: !timestamps || (begin > 0n && end >= begin),
    };
  `);
    expect(got).toEqual({
      ok: {
        error: null,
        left: "255,0,0,255",
        triangle: "0,255,0,255",
        clear: "0,0,255,255",
        label: true,
        occlusion: true,
        timestamps: true,
      },
    });
  },
  T,
);
