/*
 * C-side test of the ticoraw C API, linked against the static library by
 * scripts/c-test.sh (also under AddressSanitizer).
 *
 *   test_capi <synthetic.NEF> <width> <height> <fnv1a-hash> [real.NEF ...]
 *
 * The first file comes from `cargo run --example synthetic_nef`, which prints the width,
 * height and pixel hash that decoding it must give. Further files (for example from
 * TICORAW_CORPUS) are only checked to decode, convert and inspect cleanly.
 */
#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "ticoraw.h"

static int failures = 0;

#define CHECK(cond)                                                              \
  do {                                                                           \
    if (!(cond)) {                                                               \
      fprintf(stderr, "%s:%d: CHECK failed: %s (last error: %s)\n", __FILE__,    \
              __LINE__, #cond, tr_last_error_message());                         \
      failures++;                                                                \
    }                                                                            \
  } while (0)

static uint8_t *read_file(const char *path, size_t *len) {
  FILE *f = fopen(path, "rb");
  if (!f) {
    perror(path);
    return NULL;
  }
  uint8_t *buf = NULL;
  size_t cap = 0, n = 0;
  for (;;) {
    if (n == cap) {
      cap = cap ? cap * 2 : 1 << 20;
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
  fclose(f);
  *len = n;
  return buf;
}

static uint64_t fnv1a(const uint16_t *px, size_t count) {
  uint64_t h = 0xcbf29ce484222325ULL;
  for (size_t i = 0; i < count; i++) {
    uint8_t bytes[2] = {(uint8_t)(px[i] & 0xff), (uint8_t)(px[i] >> 8)};
    for (int b = 0; b < 2; b++) {
      h ^= bytes[b];
      h *= 0x100000001b3ULL;
    }
  }
  return h;
}

static int is_tiff(const uint8_t *d, size_t len) {
  return len >= 8 && ((memcmp(d, "II*\0", 4) == 0) || (memcmp(d, "MM\0*", 4) == 0));
}

/* Decodes, converts and inspects one file that has no expected values. */
static void check_file(const uint8_t *nef, size_t len, const char *name) {
  TrRaw *raw = NULL;
  TrStatus st = tr_decode_nef(nef, len, &raw);
  if (st != TR_STATUS_OK) {
    printf("%s: not decoded (status %d): %s\n", name, (int)st, tr_last_error_message());
    CHECK(raw == NULL);
    CHECK(st != TR_STATUS_PANIC);
    CHECK(tr_last_error_message()[0] != '\0');
    return;
  }
  size_t w = tr_raw_width(raw), h = tr_raw_height(raw);
  CHECK(w > 0 && h > 0);
  CHECK(tr_raw_pixels(raw) != NULL);
  uint16_t black[4];
  CHECK(tr_raw_black_level(raw, black) == TR_STATUS_OK);
  uint64_t hash = fnv1a(tr_raw_pixels(raw), w * h);

  TrBuffer *dng = NULL;
  CHECK(tr_nef_to_dng(nef, len, NULL, &dng) == TR_STATUS_OK);
  CHECK(is_tiff(tr_buffer_data(dng), tr_buffer_len(dng)));
  TrBuffer *json = NULL;
  CHECK(tr_inspect_json(nef, len, &json) == TR_STATUS_OK);
  CHECK(json && strstr((const char *)tr_buffer_data(json), "\"codec\":\"ticoraw\""));
  printf("%s: %s %zux%zu, pixels %016" PRIx64 ", DNG %zu bytes\n", name, tr_raw_model(raw),
         w, h, hash, tr_buffer_len(dng));
  tr_buffer_free(json);
  tr_buffer_free(dng);
  tr_raw_free(raw);
}

int main(int argc, char **argv) {
  if (argc < 5) {
    fprintf(stderr, "usage: %s <synthetic.NEF> <width> <height> <hash> [real.NEF ...]\n",
            argv[0]);
    return 2;
  }
  size_t want_w = (size_t)strtoull(argv[2], NULL, 10);
  size_t want_h = (size_t)strtoull(argv[3], NULL, 10);
  uint64_t want_hash = strtoull(argv[4], NULL, 16);

  /* Version and ABI. */
  CHECK(tr_abi_version() >= TR_ABI_VERSION);
  CHECK(tr_version() != NULL && tr_version()[0] != '\0');
  CHECK(tr_last_error_message() != NULL && tr_last_error_message()[0] == '\0');

  size_t len = 0;
  uint8_t *nef = read_file(argv[1], &len);
  if (!nef) return 1;

  /* Decode and every accessor. */
  TrRaw *raw = NULL;
  CHECK(tr_decode_nef(nef, len, &raw) == TR_STATUS_OK);
  if (!raw) return 1;
  CHECK(tr_raw_width(raw) == want_w);
  CHECK(tr_raw_height(raw) == want_h);
  CHECK(fnv1a(tr_raw_pixels(raw), want_w * want_h) == want_hash);
  CHECK(strcmp(tr_raw_make(raw), "NIKON CORPORATION") == 0);
  CHECK(strcmp(tr_raw_model(raw), "NIKON Z6_3") == 0);
  CHECK(tr_raw_orientation(raw) == 1);
  CHECK(tr_raw_white_level(raw) > 0);
  uint8_t cfa[4];
  CHECK(tr_raw_cfa_pattern(raw, cfa) == TR_STATUS_OK);
  CHECK(cfa[0] == 0 && cfa[1] == 1 && cfa[2] == 1 && cfa[3] == 2);
  uint16_t black[4];
  CHECK(tr_raw_black_level(raw, black) == TR_STATUS_OK);
  CHECK(black[0] == 1008 && black[3] == 1011);
  TrSRational matrix[9];
  CHECK(tr_raw_color_matrix(raw, matrix) == TR_STATUS_OK);
  CHECK(matrix[0].den != 0);
  TrURational neutral[3];
  CHECK(tr_raw_as_shot_neutral(raw, neutral) == TR_STATUS_OK);
  CHECK(neutral[1].num == 1 && neutral[1].den == 1);
  TrCrop crop = {7, 7, 7, 7};
  CHECK(tr_raw_crop(raw, &crop) == TR_STATUS_NOT_PRESENT);
  CHECK(crop.x == 7);
  TrSRational be;
  TrStatus st = tr_raw_baseline_exposure(raw, &be);
  CHECK(st == TR_STATUS_OK || st == TR_STATUS_NOT_PRESENT);
  CHECK(tr_raw_cfa_pattern(raw, NULL) == TR_STATUS_NULL_POINTER);
  CHECK(strcmp(tr_last_error_message(), "out is NULL") == 0);
  tr_raw_free(raw);

  /* DNG with defaults and with options. */
  TrBuffer *lossless = NULL;
  CHECK(tr_nef_to_dng(nef, len, NULL, &lossless) == TR_STATUS_OK);
  CHECK(is_tiff(tr_buffer_data(lossless), tr_buffer_len(lossless)));
  TrDngOptions opts = {0};
  opts.struct_size = sizeof opts;
  opts.compression = TR_COMPRESSION_UNCOMPRESSED;
  opts.preview = TR_PREVIEW_NONE;
  opts.original_raw_file_name = "SYN_0001.NEF";
  TrBuffer *plain = NULL;
  CHECK(tr_nef_to_dng(nef, len, &opts, &plain) == TR_STATUS_OK);
  CHECK(tr_buffer_len(plain) > want_w * want_h * 2);
  CHECK(tr_buffer_len(plain) != tr_buffer_len(lossless));
  tr_buffer_free(plain);
  tr_buffer_free(lossless);

  /* Inspect JSON: NUL-terminated, not counted in the length. */
  TrBuffer *json = NULL;
  CHECK(tr_inspect_json(nef, len, &json) == TR_STATUS_OK);
  const char *text = (const char *)tr_buffer_data(json);
  CHECK(strlen(text) == tr_buffer_len(json));
  const char *prefix = "{\"make\":\"NIKON CORPORATION\",\"model\":\"NIKON Z6_3\",";
  CHECK(strncmp(text, prefix, strlen(prefix)) == 0);
  CHECK(strstr(text, "\"compression\":{\"code\":14,") != NULL);
  tr_buffer_free(json);

  /* Failures: status, cleared output, message. */
  raw = (TrRaw *)&failures; /* any non-NULL value: must be reset */
  CHECK(tr_decode_nef(nef, len / 2, &raw) == TR_STATUS_TRUNCATED);
  CHECK(raw == NULL);
  CHECK(strncmp(tr_last_error_message(), "truncated", 9) == 0);
  CHECK(tr_decode_nef(NULL, 0, &raw) == TR_STATUS_TRUNCATED);
  CHECK(tr_decode_nef(NULL, 5, &raw) == TR_STATUS_NULL_POINTER);
  CHECK(tr_decode_nef(nef, len, NULL) == TR_STATUS_NULL_POINTER);
  CHECK(tr_decode_nef((const uint8_t *)"not a tiff, not a nef", 21, &raw) ==
        TR_STATUS_CONTAINER);
  TrBuffer *buf = (TrBuffer *)&failures;
  TrDngOptions short_opts = {0};
  short_opts.struct_size = 4;
  CHECK(tr_nef_to_dng(nef, len, &short_opts, &buf) == TR_STATUS_INVALID_ARGUMENT);
  CHECK(buf == NULL);
  opts.compression = 9;
  CHECK(tr_nef_to_dng(nef, len, &opts, &buf) == TR_STATUS_INVALID_ARGUMENT);
  CHECK(strcmp(tr_last_error_message(), "unknown compression value 9") == 0);
  opts.compression = TR_COMPRESSION_LOSSLESS_JPEG;
  opts.extra_xmp = NULL;
  opts.extra_xmp_len = 3;
  CHECK(tr_nef_to_dng(nef, len, &opts, &buf) == TR_STATUS_NULL_POINTER);

  /* NULL handles. */
  CHECK(tr_raw_width(NULL) == 0);
  CHECK(tr_raw_pixels(NULL) == NULL);
  CHECK(tr_raw_model(NULL) == NULL);
  CHECK(tr_buffer_len(NULL) == 0);
  CHECK(tr_buffer_data(NULL) == NULL);
  CHECK(tr_raw_black_level(NULL, black) == TR_STATUS_NULL_POINTER);
  tr_raw_free(NULL);
  tr_buffer_free(NULL);

  free(nef);

  for (int i = 5; i < argc; i++) {
    uint8_t *data = read_file(argv[i], &len);
    if (!data) {
      failures++;
      continue;
    }
    check_file(data, len, argv[i]);
    free(data);
  }

  if (failures) {
    fprintf(stderr, "test_capi: %d check(s) failed\n", failures);
    return 1;
  }
  printf("test_capi: all checks passed (ticoraw %s, ABI %" PRIu32 ")\n", tr_version(),
         tr_abi_version());
  return 0;
}
