#' Stream logs from the running ghqc server to the console
#'
#' Blocks the R session and prints server log output as it arrives.
#' Press Ctrl+C (or Escape in RStudio/Positron) to stop streaming.
#'
#' @param interval Seconds to wait for output before checking again.
#' @export
ghqc_log <- function(interval = 0.2) {
  proc <- .ghqc_env$proc

  if (is.null(proc)) {
    message("No ghqc server has been started this session.")
    return(invisible(NULL))
  }
  if (!proc$is_alive()) {
    message("ghqc server is not running.")
    return(invisible(NULL))
  }

  message("Streaming ghqc logs (press Ctrl+C to stop)...")
  on.exit(message("Log streaming stopped."))

  repeat {
    poll_result <- processx::poll(list(proc), as.integer(interval * 1000))
    stderr_status <- poll_result[[1]][["error"]]

    if (stderr_status == "ready") {
      lines <- proc$read_error_lines()
      if (length(lines) > 0) cat(paste0(lines, collapse = "\n"), "\n")
    }

    if (stderr_status == "eof" || !proc$is_alive()) {
      remaining <- proc$read_error()
      if (nzchar(trimws(remaining))) cat(remaining)
      message("ghqc server has stopped.")
      break
    }
  }
}
