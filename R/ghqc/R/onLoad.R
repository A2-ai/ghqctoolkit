.ghqc_env <- new.env(parent = emptyenv())

.onLoad <- function(...) {
  init_logger() |> packageStartupMessage()
}
