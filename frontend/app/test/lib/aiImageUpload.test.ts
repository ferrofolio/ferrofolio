import { describe, expect, it } from "vitest"

import { isImageFile } from "@/lib/aiImageUpload"

describe("isImageFile", () => {
  it.each([
    ["photo.jpg", "image/jpeg"],
    ["scan.png", "image/png"],
    ["IMG_0001.HEIC", ""],
    ["picture.heif", "application/octet-stream"],
  ])("accepts %s (%s)", (name, type) => {
    expect(isImageFile(new File(["x"], name, { type }))).toBe(true)
  })

  it.each([
    ["statement.pdf", "application/pdf"],
    ["export.csv", "text/csv"],
    ["heic.txt", "text/plain"],
  ])("rejects %s (%s)", (name, type) => {
    expect(isImageFile(new File(["x"], name, { type }))).toBe(false)
  })
})
