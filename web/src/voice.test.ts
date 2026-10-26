// Feature: Web renderer — voice (R26.4, R28.6)
import { describe, expect, it } from "vitest";
import { VOICE_EXT, audioPartJson, floatToPcm16, pcm16ToFloat, toBase64 } from "./voice";

describe("voice", () => {
  it("converts between float samples and 16-bit PCM, clipping", () => {
    const pcm = floatToPcm16(new Float32Array([0, 0.5, -0.5, 2, -2]));
    expect(pcm.length).toBe(10);
    const back = pcm16ToFloat(pcm);
    expect(back[1]).toBeCloseTo(0.5, 3);
    expect(back[2]).toBeCloseTo(-0.5, 3);
    expect(back[3]).toBeCloseTo(1, 3);
    expect(back[4]).toBeCloseTo(-1, 3);
  });

  it("builds the voice/v1 audio part the harness reads", () => {
    const part = audioPartJson(new Uint8Array([1, 2, 3, 4]), "web-1");
    expect(part.raw).toBe(toBase64(new Uint8Array([1, 2, 3, 4])));
    expect(part.raw).toBe("AQIDBA==");
    expect(part.mediaType).toBe("audio/pcm");
    expect(part.metadata).toEqual({
      [VOICE_EXT]: {
        seq: 1,
        attempt: { iteration: 0, n: 1 },
        format: "pcm16@24k",
        final: true,
        session: "web-1",
      },
    });
  });
});
