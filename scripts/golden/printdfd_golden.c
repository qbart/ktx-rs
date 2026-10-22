// Golden-data generator for printDFD / printDFDJSON (dev tooling, not part of
// the Rust library). For every KTX2 file given, prints the DFD as text, as
// indented JSON, as minified JSON and as text truncated by 20 bytes.
//
// Build/run via scripts/golden/gen_printdfd_golden.sh.
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

void printDFD(uint32_t* DFD, uint32_t dataSize);
void printDFDJSON(uint32_t* DFD, uint32_t dataSize, uint32_t base_indent, uint32_t indent_width, bool minified);

int main(int argc, char** argv) {
    for (int i = 1; i < argc; i++) {
        FILE* f = fopen(argv[i], "rb");
        if (!f) return 1;
        uint8_t hdr[80];
        if (fread(hdr, 1, 80, f) != 80) return 1;
        uint32_t ofs, len;
        memcpy(&ofs, hdr + 48, 4);
        memcpy(&len, hdr + 52, 4);
        uint32_t* dfd = calloc(len / 4 + 1, 4);
        fseek(f, ofs, SEEK_SET);
        if (fread(dfd, 1, len, f) != len) return 1;
        fclose(f);
        const char* name = strrchr(argv[i], '/');
        printf("== %s\n", name ? name + 1 : argv[i]);
        printDFD(dfd, len);
        printf("-- json\n");
        printDFDJSON(dfd, len, 1, 4, false);
        printf("-- minified\n");
        printDFDJSON(dfd, len, 1, 4, true);
        printf("\n-- truncated\n");
        printDFD(dfd, len > 20 ? len - 20 : 0);
        printDFDJSON(dfd, len > 20 ? len - 20 : 0, 0, 2, false);
        free(dfd);
    }
    return 0;
}
