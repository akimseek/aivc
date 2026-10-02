import type { TerminalDataPayload } from "../types";

/**
 * 把 `terminal:data` 载荷解码成字节。
 *
 * # 为什么载荷是 base64 而不是数字数组
 *
 * 数字数组（`[27,91,50,74,…]`）每个字节最多占 4 个字符，base64 是 4/3 比率
 * ——同样一段输出，跨 IPC 搬进 webview 的字符数少一半以上，接收端也不必逐元素
 * parse 成 `Uint8Array`。Rust 侧的合批压**事件数**，base64 压**单条体积**，
 * 两者是同一件事的两半（另一端见 `crates/aivc-ipc/src/terminal.rs`）。
 *
 * # 为什么认 `encoding` 标签而不是直接约定
 *
 * 载荷是跨进程契约。标签让接收端一眼知道手里是什么编码，而不是靠「收到的
 * 字符串恰好能解码」来猜。**标签不认识就丢弃**——宁可少一段输出，不可把未
 * 解码的文本当字节喂给 xterm（那会变成满屏乱码）。
 */
export function decodeTerminalData(payload: TerminalDataPayload): Uint8Array {
  if (payload.encoding !== "base64") return new Uint8Array(0);
  const binary = atob(payload.data);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}
