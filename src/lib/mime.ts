export function isJavaScriptMime(mimeType?: string): boolean {
  if (!mimeType) return false;
  const mime = mimeType.toLowerCase().split(";")[0]?.trim() ?? "";
  return (
    mime === "application/javascript" ||
    mime === "text/javascript" ||
    mime === "application/x-javascript" ||
    mime === "application/ecmascript" ||
    mime === "text/ecmascript" ||
    mime === "application/x-ecmascript"
  );
}
