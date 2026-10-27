/*
 * Converts a Nikon HE/HE* NEF to DNG with the ticoraw C API.
 *
 *   convert [--uncompressed] [--no-preview] <in.NEF> <out.dng>
 *
 * Build (after `cargo build -p ticoraw-capi --release`), for example on Linux:
 *
 *   cc -I crates/ticoraw-capi/include crates/ticoraw-capi/examples/convert.c \
 *      target/release/libticoraw_capi.a -lpthread -ldl -lm -o convert
 *
 * scripts/c-test.sh builds it with the right system libraries for each platform.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "ticoraw.h"

static uint8_t *read_file(const char *path, size_t *len) {
  FILE *f = fopen(path, "rb");
  if (!f) return NULL;
  uint8_t *buf = NULL;
  size_t cap = 0, n = 0;
  for (;;) {
    if (n == cap) {
      cap = cap ? cap * 2 : 1 << 24;
      uint8_t *grown = realloc(buf, cap);
      if (!grown) {
        free(buf);
        fclose(f);
        return NULL;
      }
      buf = grown;
    }
    size_t got = fread(buf + n, 1, cap - n, f);
    n += got;
    if (got == 0) break;
  }
  int failed = ferror(f);
  fclose(f);
  if (failed) {
    free(buf);
    return NULL;
  }
  *len = n;
  return buf;
}

/* The file name without directories, for OriginalRawFileName. */
static const char *base_name(const char *path) {
  const char *name = path;
  for (const char *p = path; *p; p++) {
    if (*p == '/' || *p == '\\') name = p + 1;
  }
  return name;
}

int main(int argc, char **argv) {
  TrDngOptions opts = {0};
  opts.struct_size = sizeof opts;
  const char *in = NULL, *out = NULL;
  for (int i = 1; i < argc; i++) {
    if (strcmp(argv[i], "--uncompressed") == 0) {
      opts.compression = TR_COMPRESSION_UNCOMPRESSED;
    } else if (strcmp(argv[i], "--no-preview") == 0) {
      opts.preview = TR_PREVIEW_NONE;
    } else if (!in) {
      in = argv[i];
    } else if (!out) {
      out = argv[i];
    } else {
      in = NULL;
      break;
    }
  }
  if (!in || !out) {
    fprintf(stderr, "usage: %s [--uncompressed] [--no-preview] <in.NEF> <out.dng>\n",
            argv[0]);
    return 2;
  }

  size_t len = 0;
  uint8_t *nef = read_file(in, &len);
  if (!nef) {
    perror(in);
    return 1;
  }
  opts.original_raw_file_name = base_name(in);

  TrBuffer *dng = NULL;
  TrStatus st = tr_nef_to_dng(nef, len, &opts, &dng);
  free(nef);
  if (st != TR_STATUS_OK) {
    fprintf(stderr, "%s: %s (status %d)\n", in, tr_last_error_message(), (int)st);
    return 1;
  }

  FILE *f = fopen(out, "wb");
  size_t n = tr_buffer_len(dng);
  int ok = f && fwrite(tr_buffer_data(dng), 1, n, f) == n;
  if (f && fclose(f) != 0) ok = 0;
  tr_buffer_free(dng);
  if (!ok) {
    perror(out);
    return 1;
  }
  printf("%s -> %s (%zu bytes)\n", in, out, n);
  return 0;
}
