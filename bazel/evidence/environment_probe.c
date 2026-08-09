#include <stdio.h>
#include <stdlib.h>
#include <string.h>

extern char **environ;

static int compare_names(const void *left, const void *right) {
  return strcmp(*(const char *const *)left, *(const char *const *)right);
}

int main(int argc, char **argv) {
  if (argc != 2) {
    return 64;
  }
  FILE *output = fopen(argv[1], "w");
  if (output == NULL) {
    return 65;
  }
  size_t count = 0;
  while (environ[count] != NULL) {
    ++count;
  }
  char **names = calloc(count == 0 ? 1 : count, sizeof(*names));
  if (names == NULL) {
    fclose(output);
    return 66;
  }
  for (size_t index = 0; index < count; ++index) {
    const char *separator = strchr(environ[index], '=');
    if (separator == NULL) {
      free(names);
      fclose(output);
      return 67;
    }
    const size_t length = (size_t)(separator - environ[index]);
    names[index] = strndup(environ[index], length);
    if (names[index] == NULL) {
      for (size_t previous = 0; previous < index; ++previous) {
        free(names[previous]);
      }
      free(names);
      fclose(output);
      return 68;
    }
  }
  qsort(names, count, sizeof(*names), compare_names);
  fputs("environment-names-only-v1\n", output);
  for (size_t index = 0; index < count; ++index) {
    fputs(names[index], output);
    fputc('\n', output);
    free(names[index]);
  }
  free(names);
  return fclose(output) == 0 ? 0 : 69;
}
