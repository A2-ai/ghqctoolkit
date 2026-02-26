.ghqc_env <- new.env(parent = emptyenv())

#' @export
ghqc <- function(directory = ".", port = NULL) {
  ghqc_stop()

  directory <- here::here(directory)
  port <- if (is.null(port)) random_port()

  proc <- callr::r_bg(
    function(port, directory) ghqc:::run(port, directory),
    args = list(port = port, directory = directory),
    stderr = "|",
    supervise = TRUE
  )

  if (!wait_for_server(port)) {
    err <- proc$read_error()
    if (nzchar(trimws(err))) {
      stop("ghqc server failed to start:\n", err)
    } else {
      stop("ghqc server did not start within the timeout period")
    }
  }

  .ghqc_env$proc <- proc
  .ghqc_env$port <- port

  utils::browseURL(glue::glue("http://localhost:{port}"))
}

wait_for_server <- function(port, timeout = 15) {
  deadline <- Sys.time() + timeout
  while (Sys.time() < deadline) {
    ready <- tryCatch(
      suppressWarnings({
        con <- socketConnection("0.0.0.0", port, timeout = 0.5, open = "r+")
        close(con)
        TRUE
      }),
      error = function(e) FALSE
    )
    if (ready) {
      return(invisible(TRUE))
    }
    Sys.sleep(0.1)
  }
  invisible(FALSE)
}


#' Stop a running ghqc background server
#' @export
ghqc_stop <- function() {
  proc <- .ghqc_env$proc
  if (is.null(proc)) {
    message("No background ghqc server is running.")
    return(invisible(NULL))
  }
  if (!proc$is_alive()) {
    message("ghqc server has already stopped.")
    .ghqc_env$proc <- NULL
    return(invisible(NULL))
  }
  proc$kill()
  .ghqc_env$proc <- NULL
  message("ghqc server stopped.")
  invisible(NULL)
}

#' Check the status of the ghqc background server
#' @export
ghqc_status <- function() {
  port <- .ghqc_env$port

  if (is.null(port)) {
    message("No ghqc server has been started this session.")
    return(invisible(NULL))
  }

  proc <- .ghqc_env$proc
  url <- glue::glue("http://localhost:{port}")

  if (proc$is_alive()) {
    message(glue::glue("ghqc server is running at {url}"))
  } else {
    message(glue::glue("ghqc server has stopped (was at {url})"))
  }

  invisible(url)
}

#' Reopen the ghqc viewer for an already-running server
#' @export
ghqc_reconnect <- function() {
  port <- .ghqc_env$port
  if (is.null(port)) {
    message(
      "No ghqc server has been started this session. Use ghqc() to start one."
    )
    return(invisible(NULL))
  }

  proc <- .ghqc_env$proc
  if (!proc$is_alive()) {
    message("ghqc server has stopped. Use ghqc() to start a new one.")
    return(invisible(NULL))
  }

  url <- glue::glue("http://localhost:{port}")
  utils::browseURL(url)
  invisible(url)
}
