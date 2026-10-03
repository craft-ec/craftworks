// IMAGE STUDIO, a capability: the ONE way an image is uploaded, from any app (an editor's 🖼, a 📎, Drive, and the
// Images app to come) — as `video-studio` is for videos and audio: put (`files`) with its THUMBNAIL (at most 320 px,
// WebP, kept in the reference: what every list and cover shows). `kinds` names it the image domain's maker.
//
//   const studio = await ctx.require("image-studio");
//   const ref = await studio.make(file, { space, public, app, onProgress })   // the image's reference, with `preview`
//   await studio.thumbnail(file)                                              // a data: URL, or null
export async function start(ctx) {
  const files = await ctx.require("files");

  async function thumbnail(file) {
    try {
      const bmp = await createImageBitmap(file);
      const s = Math.min(1, 320 / Math.max(bmp.width, bmp.height));
      const c = Object.assign(document.createElement("canvas"), { width: Math.max(1, Math.round(bmp.width * s)), height: Math.max(1, Math.round(bmp.height * s)) });
      c.getContext("2d").drawImage(bmp, 0, 0, c.width, c.height);
      const blob = await new Promise(r => c.toBlob(r, "image/webp", 0.75));
      if (!blob || blob.size > 48 * 1024) return null;
      return await new Promise(r => {
        const fr = new FileReader();
        fr.onload = () => r(fr.result);
        fr.readAsDataURL(blob);
      });
    } catch {
      return null;
    }
  }

  async function make(file, { space = null, public: pub = false, app = "images", onProgress = () => {} } = {}) {
    const preview = await thumbnail(file);
    const up = await files.put(file, { space, public: !!pub, app, onProgress: p => onProgress({ stage: "uploading", p: p.done / Math.max(1, p.size) }) });
    return { ...up, name: file.name, ...(preview ? { preview } : {}) };
  }
  return { make, thumbnail };
}
