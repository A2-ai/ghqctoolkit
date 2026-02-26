# Derived from httpuv::randomPort() (https://github.com/rstudio/httpuv).
# The logic and unsafe port list are identical. The only difference is how port
# availability is tested: httpuv uses its own startServer() (a C-level httpuv
# server) whereas here we use base R's serverSocket(), which binds a TCP socket
# without requiring the httpuv package. This avoids a heavy dependency for what
# is otherwise a single utility function.
random_port <- function(min = 1024L, max = 49151L, n = 20) {
  min <- max(1L, min)
  max <- min(max, 65535L)
  valid_ports <- setdiff(seq.int(min, max), .unsafe_ports)

  n <- min(n, length(valid_ports))
  try_ports <- if (n < 2) valid_ports else sample(valid_ports, n)

  for (port in try_ports) {
    if (.is_port_available(port)) return(port)
  }

  stop("Cannot find an available port.")
}

.is_port_available <- function(port) {
  tryCatch({
    con <- serverSocket(port)
    close(con)
    TRUE
  }, error = function(e) FALSE)
}

# Ports considered unsafe by Chrome
# http://superuser.com/questions/188058/which-ports-are-considered-unsafe-on-chrome
.unsafe_ports <- c(
  1, 7, 9, 11, 13, 15, 17, 19, 20, 21, 22, 23, 25, 37, 42, 43, 53, 77, 79,
  87, 95, 101, 102, 103, 104, 109, 110, 111, 113, 115, 117, 119, 123, 135,
  139, 143, 179, 389, 427, 465, 512, 513, 514, 515, 526, 530, 531, 532, 540,
  548, 556, 563, 587, 601, 636, 993, 995, 2049, 3659, 4045, 6000, 6665, 6666,
  6667, 6668, 6669, 6697
)
