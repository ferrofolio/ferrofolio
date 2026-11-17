const HEIC_EXTENSIONS = [".heic", ".heif"]

export const isImageFile = (file: File) => {
  const name = file.name.toLowerCase()
  return (
    file.type.startsWith("image/") ||
    HEIC_EXTENSIONS.some(extension => name.endsWith(extension))
  )
}
