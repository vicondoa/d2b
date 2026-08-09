#include <stdio.h>

extern char **environ;

int main(int argc, char **argv) {
  if (argc != 2) {
    return 64;
  }
  FILE *output = fopen(argv[1], "w");
  if (output == NULL) {
    return 65;
  }
  for (char **entry = environ; entry != NULL && *entry != NULL; ++entry) {
    fputs(*entry, output);
    fputc('\n', output);
  }
  return fclose(output) == 0 ? 0 : 66;
}
