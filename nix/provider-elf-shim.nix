{ pkgs
, name
, interpreterPkg
, interpreterPath
, program
, extraArgs ? [ ]
}:

let
  lib = pkgs.lib;

  fail = message:
    throw "d2b.lib.buildProviderElfShim: ${message}";

  asString = label: value:
    let
      result = builtins.tryEval ("${value}");
    in
    if result.success then result.value
    else fail "${label} must be a path, store path, or derivation output";

  forbiddenInFixedString = value:
    lib.any (needle: lib.hasInfix needle value) [
      "\u0000"
      "\n"
      "\r"
      "\t"
    ];

  validName =
    builtins.isString name
    && builtins.stringLength name >= 1
    && builtins.stringLength name <= 64
    && builtins.match "^[a-z][a-z0-9-]*$" name != null;

  validRelativePath = value:
    builtins.isString value
    && builtins.stringLength value >= 1
    && !(lib.hasPrefix "/" value)
    && !(forbiddenInFixedString value)
    && !(lib.elem ".." (lib.splitString "/" value))
    && lib.all (part: part != "") (lib.splitString "/" value);

  interpreterOutputPath = asString "interpreterPkg" interpreterPkg;
  programPath = asString "program" program;

  storePathInfo = label: value:
    let
      parts = lib.filter (part: part != "") (lib.splitString "/" value);
      valid =
        builtins.length parts >= 3
        && builtins.elemAt parts 0 == "nix"
        && builtins.elemAt parts 1 == "store";
    in
    if !valid then
      fail "${label} must resolve to a path below /nix/store"
    else
      {
        root = "/nix/store/${builtins.elemAt parts 2}";
        relative = lib.concatStringsSep "/" (lib.drop 3 parts);
      };

  interpreterInfo = storePathInfo "interpreterPkg" interpreterOutputPath;
  interpreterArgv0 = lib.last (lib.splitString "/" interpreterPath);
  programStorePathInfo = storePathInfo "program" programPath;
  programInfo =
    if programStorePathInfo.relative == "" then
      {
        root = programPath;
        relative = ".";
        rootIsFile = true;
      }
    else
      (programStorePathInfo // { rootIsFile = false; });

  cString = value: builtins.toJSON value;
  extraArgsC =
    if extraArgs == [ ] then
      "  0"
    else
      lib.concatStringsSep ",\n"
        (map (value: "  ${cString value}") extraArgs);

  verifierSource = builtins.toFile "d2b-provider-elf-shim-verify.c" ''
    #define _GNU_SOURCE
    #include <errno.h>
    #include <fcntl.h>
    #include <limits.h>
    #include <linux/openat2.h>
    #include <stdint.h>
    #include <stdio.h>
    #include <stdlib.h>
    #include <string.h>
    #include <sys/stat.h>
    #include <sys/syscall.h>
    #include <sys/types.h>
    #include <unistd.h>

    #ifndef O_PATH
    #define O_PATH 010000000
    #endif
    #ifndef RESOLVE_BENEATH
    #define RESOLVE_BENEATH 0x08
    #endif
    #ifndef RESOLVE_NO_MAGICLINKS
    #define RESOLVE_NO_MAGICLINKS 0x02
    #endif
    #ifndef RESOLVE_NO_SYMLINKS
    #define RESOLVE_NO_SYMLINKS 0x04
    #endif
    #ifndef ELFCLASS64
    #define ELFCLASS64 2
    #endif
    #ifndef ELFDATA2LSB
    #define ELFDATA2LSB 1
    #endif
    #ifndef ELFDATA2MSB
    #define ELFDATA2MSB 2
    #endif
    #ifndef ET_EXEC
    #define ET_EXEC 2
    #endif
    #ifndef ET_DYN
    #define ET_DYN 3
    #endif

    #define MAX_COMPONENTS 256
    #define MAX_COMPONENT_BYTES 255
    #define MAX_PATH_BYTES 4096
    #define MAX_LINKS 8

    struct path_vector {
      char values[MAX_COMPONENTS][MAX_COMPONENT_BYTES + 1];
      size_t length;
    };

    struct resolved_path {
      char full[MAX_PATH_BYTES];
      char relative[MAX_PATH_BYTES];
    };

    static void fail_with(const char *label, const char *reason) {
      fprintf(stderr, "d2b-provider-elf-shim: %s: %s\n", label, reason);
      exit(1);
    }

    static void check_size(size_t value, const char *label) {
      if (value >= MAX_PATH_BYTES)
        fail_with(label, "path too long");
    }

    static void vector_add(
        struct path_vector *vector,
        const char *value,
        const char *label) {
      size_t length = strlen(value);
      if (length == 0 || length > MAX_COMPONENT_BYTES)
        fail_with(label, "invalid path component");
      if (strcmp(value, "..") == 0)
        fail_with(label, "parent path component is not allowed");
      if (strcmp(value, ".") == 0)
        return;
      if (vector->length == MAX_COMPONENTS)
        fail_with(label, "too many path components");
      memcpy(vector->values[vector->length], value, length + 1);
      vector->length++;
    }

    static void parse_relative(
        const char *value,
        struct path_vector *vector,
        const char *label) {
      char copy[MAX_PATH_BYTES];
      char *save = NULL;
      char *part;

      if (value[0] == '/')
        fail_with(label, "absolute path is not allowed");
      check_size(strlen(value), label);
      memcpy(copy, value, strlen(value) + 1);
      vector->length = 0;
      part = strtok_r(copy, "/", &save);
      while (part != NULL) {
        vector_add(vector, part, label);
        part = strtok_r(NULL, "/", &save);
      }
      if (vector->length == 0)
        fail_with(label, "empty relative path");
    }

    static void parse_link_target(
        const char *value,
        struct path_vector *vector,
        const char *label) {
      char copy[MAX_PATH_BYTES];
      char *save = NULL;
      char *part;

      if (value[0] == '/')
        fail_with(label, "absolute symlink target is not allowed");
      check_size(strlen(value), label);
      memcpy(copy, value, strlen(value) + 1);
      vector->length = 0;
      part = strtok_r(copy, "/", &save);
      while (part != NULL) {
        vector_add(vector, part, label);
        part = strtok_r(NULL, "/", &save);
      }
      if (vector->length == 0)
        fail_with(label, "empty symlink target");
    }

    static void remove_first(struct path_vector *vector) {
      if (vector->length > 1) {
        memmove(
            vector->values[0],
            vector->values[1],
            (vector->length - 1) * sizeof(vector->values[0]));
      }
      vector->length--;
    }

    static void prepend_target(
        struct path_vector *pending,
        const struct path_vector *target,
        const char *label) {
      struct path_vector replacement = { 0 };
      size_t index;

      if (target->length + pending->length - 1 > MAX_COMPONENTS)
        fail_with(label, "too many path components");
      for (index = 0; index < target->length; index++)
        vector_add(&replacement, target->values[index], label);
      for (index = 1; index < pending->length; index++)
        vector_add(&replacement, pending->values[index], label);
      *pending = replacement;
    }

    static size_t make_candidate(
        const struct path_vector *current,
        const char *next,
        char *out,
        const char *label) {
      size_t offset = 0;
      size_t index;

      for (index = 0; index < current->length; index++) {
        size_t length = strlen(current->values[index]);
        if (offset != 0)
          out[offset++] = '/';
        check_size(offset + length, label);
        memcpy(out + offset, current->values[index], length);
        offset += length;
      }
      if (offset != 0)
        out[offset++] = '/';
      check_size(offset + strlen(next), label);
      memcpy(out + offset, next, strlen(next));
      offset += strlen(next);
      out[offset] = '\0';
      return offset;
    }

    static size_t make_relative(
        const struct path_vector *current,
        const char *last,
        char *out,
        const char *label) {
      return make_candidate(current, last, out, label);
    }

    static int open_beneath(int rootfd, const char *path, uint64_t flags) {
      struct open_how how = {
        .flags = flags,
        .resolve = RESOLVE_BENEATH | RESOLVE_NO_MAGICLINKS,
      };
      return (int)syscall(SYS_openat2, rootfd, path, &how, sizeof(how));
    }

    static int is_host_little_endian(void) {
    #if __BYTE_ORDER__ == __ORDER_LITTLE_ENDIAN__
      return 1;
    #else
      return 0;
    #endif
    }

    static void check_elf(
        int fd,
        mode_t mode,
        const char *label) {
      unsigned char header[18];
      size_t offset = 0;
      ssize_t count;
      uint16_t type;
      int expected_data;

      if ((mode & 0111) == 0)
        fail_with(label, "interpreter is not executable");
      while (offset < sizeof(header)) {
        count = read(fd, header + offset, sizeof(header) - offset);
        if (count < 0) {
          if (errno == EINTR)
            continue;
          fail_with(label, "interpreter header could not be read");
        }
        if (count == 0)
          fail_with(label, "interpreter is shorter than an ELF header");
        offset += (size_t)count;
      }
      if (memcmp(header, "\x7f" "ELF", 4) != 0)
        fail_with(label, "interpreter is not ELF");
      if (header[4] != ELFCLASS64)
        fail_with(label, "interpreter is not ELF64");
      expected_data = is_host_little_endian() ? ELFDATA2LSB : ELFDATA2MSB;
      if (header[5] != expected_data)
        fail_with(label, "interpreter byte order differs from the host");
      if (header[6] != 1)
        fail_with(label, "interpreter ELF version is not current");
      if (is_host_little_endian())
        type = (uint16_t)header[16] | ((uint16_t)header[17] << 8);
      else
        type = ((uint16_t)header[16] << 8) | (uint16_t)header[17];
      if (type != ET_EXEC && type != ET_DYN)
        fail_with(label, "interpreter ELF type is not ET_EXEC or ET_DYN");
    }

    static void read_link(
        int rootfd,
        const char *candidate,
        char *target,
        size_t target_size,
        const char *label) {
      ssize_t length = readlinkat(rootfd, candidate, target, target_size - 1);
      if (length < 0)
        fail_with(label, "symlink target could not be read");
      if ((size_t)length >= target_size - 1)
        fail_with(label, "symlink target is too long");
      target[length] = '\0';
    }

    static void append_text(
        char *out,
        size_t *offset,
        const char *text,
        const char *label) {
      size_t length = strlen(text);
      check_size(*offset + length, label);
      memcpy(out + *offset, text, length);
      *offset += length;
      out[*offset] = '\0';
    }

    static void build_full_path(
        const char *root,
        const char *relative,
        char *out,
        const char *label) {
      size_t offset = 0;
      out[0] = '\0';
      append_text(out, &offset, root, label);
      append_text(out, &offset, "/", label);
      append_text(out, &offset, relative, label);
    }

    static void resolve_file(
        const char *root,
        const char *relative,
        int require_elf,
        const char *label,
        int root_is_file,
        struct resolved_path *result) {
      int rootfd;
      struct path_vector current = { 0 };
      struct path_vector pending;
      int links = 0;

      if (root_is_file) {
        struct stat status;

        rootfd = open(
            root,
            O_RDONLY | O_NONBLOCK | O_CLOEXEC | O_NOFOLLOW);
        if (rootfd < 0)
          fail_with(label, "program file could not be opened");
        if (fstat(rootfd, &status) < 0 || !S_ISREG(status.st_mode))
          fail_with(label, "program path terminus is not a regular file");
        if (require_elf)
          check_elf(rootfd, status.st_mode, label);
        check_size(strlen(root), label);
        memcpy(result->full, root, strlen(root) + 1);
        result->relative[0] = '\0';
        close(rootfd);
        return;
      }

      parse_relative(relative, &pending, label);
      rootfd = open(root, O_PATH | O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW);
      if (rootfd < 0)
        fail_with(label, "output root could not be opened");

      while (pending.length != 0) {
        char candidate[MAX_PATH_BYTES];
        int fd;
        struct stat status;

        make_candidate(&current, pending.values[0], candidate, label);
        fd = open_beneath(
            rootfd,
            candidate,
            O_PATH | O_NOFOLLOW | O_CLOEXEC);
        if (fd < 0)
          fail_with(label, "path resolution failed");
        if (fstat(fd, &status) < 0)
          fail_with(label, "fstat failed");

        if (S_ISLNK(status.st_mode)) {
          char target[MAX_PATH_BYTES];
          struct path_vector target_vector;

          if (++links > MAX_LINKS)
            fail_with(label, "symlink chain exceeds eight links");
          read_link(rootfd, candidate, target, sizeof(target), label);
          parse_link_target(target, &target_vector, label);
          prepend_target(&pending, &target_vector, label);
          close(fd);
          continue;
        }

        if (S_ISDIR(status.st_mode)) {
          if (current.length == MAX_COMPONENTS)
            fail_with(label, "too many path components");
          memcpy(
              current.values[current.length],
              pending.values[0],
              sizeof(current.values[current.length]));
          current.length++;
          remove_first(&pending);
          close(fd);
          continue;
        }

        if (pending.length != 1)
          fail_with(label, "path component is not a directory");
        if (!S_ISREG(status.st_mode))
          fail_with(label, "path terminus is not a regular file");

        if (require_elf) {
          int readable;
          readable = open_beneath(
              rootfd,
              candidate,
              O_RDONLY | O_NONBLOCK | O_CLOEXEC | O_NOFOLLOW);
          if (readable < 0)
            fail_with(label, "interpreter could not be opened for reading");
          if (fstat(readable, &status) < 0 || !S_ISREG(status.st_mode))
            fail_with(label, "interpreter is not a regular file");
          check_elf(readable, status.st_mode, label);
          close(readable);
        }

        make_relative(
            &current,
            pending.values[0],
            result->relative,
            label);
        build_full_path(root, result->relative, result->full, label);
        close(fd);
        close(rootfd);
        return;
      }

      close(rootfd);
      fail_with(label, "path did not terminate at a regular file");
    }

    static void write_all(int fd, const char *data, size_t length) {
      size_t offset = 0;
      while (offset < length) {
        ssize_t count = write(fd, data + offset, length - offset);
        if (count < 0) {
          if (errno == EINTR)
            continue;
          fail_with("header", "header could not be written");
        }
        offset += (size_t)count;
      }
    }

    static void write_c_string(int fd, const char *value) {
      static const char hex[] = "0123456789";
      char escaped[5];
      size_t index;

      write_all(fd, "\"", 1);
      for (index = 0; value[index] != '\0'; index++) {
        unsigned char byte = (unsigned char)value[index];
        if (byte >= 0x20 && byte <= 0x7e
            && byte != '\\' && byte != '"' && byte != '?') {
          write_all(fd, (const char *)&value[index], 1);
        } else {
          escaped[0] = '\\';
          escaped[1] = '0' + ((byte >> 6) & 07);
          escaped[2] = '0' + ((byte >> 3) & 07);
          escaped[3] = '0' + (byte & 07);
          escaped[4] = '\0';
          write_all(fd, escaped, 4);
        }
      }
      write_all(fd, "\"\n", 2);
      (void)hex;
    }

    static void write_macro(int fd, const char *name, const char *value) {
      char line[128];
      int length = snprintf(line, sizeof(line), "#define %s ", name);
      if (length < 0 || (size_t)length >= sizeof(line))
        fail_with("header", "macro name is too long");
      write_all(fd, line, (size_t)length);
      write_c_string(fd, value);
    }

    static void emit_header(
        const char *interpreter_root,
        const char *interpreter_relative,
        const char *interpreter_argv0,
        const char *program_root,
        const char *program_relative,
        int program_root_is_file,
        const char *output) {
      struct resolved_path interpreter;
      struct resolved_path program;
      const char *slash;
      char directory[MAX_PATH_BYTES];
      int fd;
      size_t directory_length;

      resolve_file(
          interpreter_root,
          interpreter_relative,
          1,
          "interpreter",
          0,
          &interpreter);
      resolve_file(
          program_root,
          program_relative,
          0,
          "program",
          program_root_is_file,
          &program);

      slash = strrchr(interpreter.full, '/');
      if (slash == NULL || slash == interpreter.full)
        fail_with("interpreter", "resolved path has no directory");
      directory_length = (size_t)(slash - interpreter.full);
      if (directory_length >= sizeof(directory))
        fail_with("interpreter", "resolved directory is too long");
      memcpy(directory, interpreter.full, directory_length);
      directory[directory_length] = '\0';

      fd = open(output, O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0644);
      if (fd < 0)
        fail_with("header", "configuration header could not be opened");
      write_macro(fd, "D2B_INTERPRETER_DIR", directory);
      write_macro(fd, "D2B_INTERPRETER_BASENAME", slash + 1);
      write_macro(fd, "D2B_INTERPRETER_ARGV0", interpreter_argv0);
      write_macro(fd, "D2B_PROGRAM_PATH", program.full);
      close(fd);
    }

    int main(int argc, char **argv) {
      if (argc != 9 || strcmp(argv[1], "--emit-header") != 0) {
        fprintf(stderr, "d2b-provider-elf-shim: invalid verifier invocation\n");
        return 2;
      }
      emit_header(
          argv[2],
          argv[3],
          argv[4],
          argv[5],
          argv[6],
          atoi(argv[7]),
          argv[8]);
      return 0;
    }
  '';

  shimSource = builtins.toFile "d2b-provider-elf-shim.c" ''
    #define _GNU_SOURCE
    #include <errno.h>
    #include <fcntl.h>
    #include <linux/openat2.h>
    #include <stdint.h>
    #include <stdlib.h>
    #include <string.h>
    #include <sys/stat.h>
    #include <sys/syscall.h>
    #include <unistd.h>

    #ifndef O_PATH
    #define O_PATH 010000000
    #endif
    #ifndef RESOLVE_BENEATH
    #define RESOLVE_BENEATH 0x08
    #endif
    #ifndef RESOLVE_NO_MAGICLINKS
    #define RESOLVE_NO_MAGICLINKS 0x02
    #endif
    #ifndef RESOLVE_NO_SYMLINKS
    #define RESOLVE_NO_SYMLINKS 0x04
    #endif
    #ifndef AT_EMPTY_PATH
    #define AT_EMPTY_PATH 0x1000
    #endif

    #include "provider-elf-shim-config.h"

    #define D2B_EXTRA_ARG_COUNT ${toString (builtins.length extraArgs)}

    #if D2B_EXTRA_ARG_COUNT > 0
    static const char *const d2b_extra_args[] = {
    ${extraArgsC}
    };
    #endif

    extern char **environ;

    static int d2b_openat2(
        int directory,
        const char *path,
        uint64_t flags,
        uint64_t resolve) {
      struct open_how how = {
        .flags = flags,
        .resolve = resolve,
      };
      return (int)syscall(
          SYS_openat2,
          directory,
          path,
          &how,
          sizeof(how));
    }

    static int fail(const char *message) {
      ssize_t ignored;
      ignored = write(STDERR_FILENO, message, strlen(message));
      (void)ignored;
      ignored = write(STDERR_FILENO, "\n", 1);
      (void)ignored;
      return 126;
    }

    int main(int argc, char **argv) {
      int directory;
      int interpreter;
      struct stat status;
      size_t caller_args;
      size_t total_args;
      size_t index;
      char **child_argv;
      int error;

      directory = d2b_openat2(
          AT_FDCWD,
          D2B_INTERPRETER_DIR,
          O_PATH | O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW,
          RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS);
      if (directory < 0)
        return fail("d2b-provider-elf-shim: interpreter directory unavailable");

      interpreter = d2b_openat2(
          directory,
          D2B_INTERPRETER_BASENAME,
          O_PATH | O_CLOEXEC | O_NOFOLLOW,
          RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS);
      if (interpreter < 0) {
        close(directory);
        return fail("d2b-provider-elf-shim: interpreter resolution refused");
      }
      if (fstat(interpreter, &status) < 0 || !S_ISREG(status.st_mode)) {
        close(interpreter);
        close(directory);
        return fail("d2b-provider-elf-shim: interpreter is not regular");
      }
      if ((status.st_mode & 0111) == 0) {
        close(interpreter);
        close(directory);
        return fail("d2b-provider-elf-shim: interpreter is not executable");
      }

      caller_args = argc > 1 ? (size_t)argc - 1 : 0;
      if (caller_args > SIZE_MAX - D2B_EXTRA_ARG_COUNT - 3) {
        close(interpreter);
        close(directory);
        return fail("d2b-provider-elf-shim: argument vector is too large");
      }
      total_args = 3 + D2B_EXTRA_ARG_COUNT + caller_args;
      child_argv = calloc(total_args, sizeof(*child_argv));
      if (child_argv == NULL) {
        close(interpreter);
        close(directory);
        return fail("d2b-provider-elf-shim: argument vector allocation failed");
      }

      child_argv[0] = (char *)D2B_INTERPRETER_ARGV0;
      child_argv[1] = (char *)D2B_PROGRAM_PATH;
    #if D2B_EXTRA_ARG_COUNT > 0
      for (index = 0; index < D2B_EXTRA_ARG_COUNT; index++)
        child_argv[2 + index] = (char *)d2b_extra_args[index];
    #endif
      for (index = 1; index < (size_t)argc; index++)
        child_argv[2 + D2B_EXTRA_ARG_COUNT + index - 1] = argv[index];
      child_argv[total_args - 1] = NULL;

      error = (int)syscall(
          SYS_execveat,
          interpreter,
          "",
          child_argv,
          environ,
          AT_EMPTY_PATH);
      error = error < 0 ? errno : 0;
      free(child_argv);
      close(interpreter);
      close(directory);
      if (error != 0) {
        if (error == ENOEXEC)
          return fail("d2b-provider-elf-shim: interpreter format rejected");
        if (error == ENOENT)
          return fail("d2b-provider-elf-shim: interpreter unavailable");
        if (error == EACCES)
          return fail("d2b-provider-elf-shim: interpreter permission denied");
        return fail("d2b-provider-elf-shim: interpreter execution failed");
      }
      return 127;
    }
  '';
in
assert validName
  || fail "name must match ^[a-z][a-z0-9-]*$ and be at most 64 bytes";
assert validRelativePath interpreterPath
  || fail "interpreterPath must be a non-empty relative path without empty or parent components";
assert interpreterInfo.relative == ""
  || fail "interpreterPkg must name the root of one Nix store output";
assert builtins.isList extraArgs
  || fail "extraArgs must be a list of strings";
assert lib.all (value: builtins.isString value && !(forbiddenInFixedString value)) extraArgs
  || fail "extraArgs must contain only strings without NUL or control separators";
pkgs.stdenv.mkDerivation {
  pname = "d2b-provider-elf-shim-${name}";
  version = "1";
  dontUnpack = true;
  strictDeps = true;
  nativeBuildInputs = [ pkgs.binutils ];

  buildPhase = ''
    runHook preBuild
    cp ${verifierSource} provider-elf-shim-verify.c
    cp ${shimSource} provider-elf-shim.c
    $CC -std=c11 -O2 -Wall -Wextra -Werror \
      provider-elf-shim-verify.c -o provider-elf-shim-verify
    ./provider-elf-shim-verify --emit-header \
      ${lib.escapeShellArg interpreterInfo.root} \
      ${lib.escapeShellArg interpreterPath} \
      ${lib.escapeShellArg interpreterArgv0} \
      ${lib.escapeShellArg programInfo.root} \
      ${lib.escapeShellArg programInfo.relative} \
      ${if programInfo.rootIsFile then "1" else "0"} \
      provider-elf-shim-config.h
    $CC -std=c11 -O2 -Wall -Wextra -Werror -fPIE -pie \
      -Wl,-z,relro -Wl,-z,now \
      -include provider-elf-shim-config.h \
      provider-elf-shim.c -o provider-elf-shim
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    install -Dm755 provider-elf-shim \
      "$out/bin/${name}"
    runHook postInstall
  '';

  postInstall = ''
    header="$(${pkgs.binutils}/bin/readelf -h "$out/bin/${name}")"
    printf '%s\n' "$header" | grep -Eq 'Class:[[:space:]]+ELF64'
    printf '%s\n' "$header" | grep -Eq 'Type:[[:space:]]+(EXEC|DYN)'
  '';

  passthru.providerElfShim = {
    inherit name interpreterPath interpreterArgv0 extraArgs;
    interpreterOutput = interpreterOutputPath;
    program = programPath;
  };

  meta.mainProgram = name;
}
