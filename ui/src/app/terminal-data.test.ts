/**
 * @vitest-environment jsdom
 */
import { describe, expect, it } from "vitest";

import { decodeTerminalData } from "./terminal-data";
import type { TerminalDataPayload } from "../types";

/** 构造一条 `terminal:data` 载荷：字节 → base64（btoa 在 jsdom 与 webview 都有）。 */
const payload = (bytes: number[]): TerminalDataPayload => ({
  terminalId: "t1",
  encoding: "base64",
  data: btoa(String.fromCharCode(...bytes))
});

describe("decodeTerminalData", () => {
  it("base64 解码回原始字节（控制序列与高位字节保真）", () => {
    const raw = [0x1b, 0x5b, 0x36, 0x6e, 0x00, 0xff, 0x80, 0xa5];
    expect(Array.from(decodeTerminalData(payload(raw)))).toEqual(raw);
  });

  it("UTF-8 多字节字符按字节保真（不经文本往返）", () => {
    const raw = Array.from(new TextEncoder().encode("中文 ok"));
    expect(Array.from(decodeTerminalData(payload(raw)))).toEqual(raw);
  });

  it("空输出解码为空字节", () => {
    expect(decodeTerminalData(payload([]))).toHaveLength(0);
  });

  it("不认识的 encoding 标签整条丢弃——不猜内容", () => {
    const rogue = {
      terminalId: "t1",
      encoding: "hex",
      data: "1b5b366e"
    } as unknown as TerminalDataPayload;
    expect(decodeTerminalData(rogue)).toHaveLength(0);
  });
});
