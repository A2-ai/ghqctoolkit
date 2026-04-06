use std::{
    collections::{BTreeMap, hash_map::DefaultHasher},
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use lopdf::{Bookmark, Document, Object, ObjectId};
use tempfile::tempdir;
use typst::{
    diag::{Severity, SourceDiagnostic},
    ecow::EcoVec,
};
use typst_pdf::PdfOptions;

use crate::{DiskCache, record::images::HttpDownloader, record::typst::TypstWorld};

/// Create a staging directory for record generation
///
/// Creates a unique staging directory in the system temp folder using a hash.
/// This directory is used to store downloaded images, the logo, and the rendered template.
pub fn create_staging_dir() -> Result<PathBuf, RenderError> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    let mut hasher = DefaultHasher::new();
    timestamp.hash(&mut hasher);
    let hash = hasher.finish();

    let staging_dir = std::env::temp_dir().join(format!("ghqc-render-{:x}", hash));
    std::fs::create_dir_all(&staging_dir)?;

    log::debug!("Created staging directory: {}", staging_dir.display());
    Ok(staging_dir)
}

#[derive(Debug, Clone)]
pub struct QCContext {
    file: PathBuf,
    position: ContextPosition,
}

impl QCContext {
    pub fn new(path: impl AsRef<Path>, position: ContextPosition) -> Self {
        Self {
            file: path.as_ref().to_path_buf(),
            position,
        }
    }

    pub fn file(&self) -> &Path {
        &self.file
    }

    pub fn position(&self) -> ContextPosition {
        self.position
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ContextPosition {
    Prepend,
    Append,
}

/// Render a Typst document to PDF using the typst library
///
/// # Arguments
/// * `record_str` - The Typst document content to render
/// * `path` - The output path for the rendered PDF
/// * `staging_dir` - The staging directory containing the template and assets (images, logo)
/// * `qc_context` - Optional context files to prepend/append to the PDF
/// * `cache` - Optional disk cache for typst package caching
/// * `http` - HTTP downloader for fetching Typst packages
///
/// # Returns
/// * `Ok(())` - If rendering succeeded
/// * `Err(RenderError)` - If rendering failed
///
/// # Example
/// ```no_run
/// use std::path::Path;
/// use ghqctoolkit::{render, QCContext, ContextPosition, create_staging_dir, UreqDownloader};
///
/// let report = "#set document(title: \"My Report\")\n= Hello World";
/// let staging_dir = create_staging_dir().unwrap();
/// render(report, Path::new("output/my-report.pdf"), &staging_dir, &[], None, &UreqDownloader::new()).unwrap();
/// ```
pub fn render(
    record_str: &str,
    path: impl AsRef<Path>,
    staging_dir: impl AsRef<Path>,
    qc_context: &[QCContext],
    cache: Option<&DiskCache>,
    http: &(impl HttpDownloader + Clone + 'static),
) -> Result<(), RenderError> {
    let staging_dir = staging_dir.as_ref();

    // Run the actual render logic, capturing the result
    let result = render_inner(
        record_str,
        path.as_ref(),
        staging_dir,
        qc_context,
        cache,
        http,
    );

    // Always cleanup staging directory, regardless of success or failure
    if let Err(e) = std::fs::remove_dir_all(staging_dir) {
        log::warn!(
            "Failed to cleanup staging directory {}: {}",
            staging_dir.display(),
            e
        );
    }

    result
}

fn render_inner(
    record_str: &str,
    output_path: &Path,
    staging_dir: &Path,
    qc_context: &[QCContext],
    cache: Option<&DiskCache>,
    http: &(impl HttpDownloader + Clone + 'static),
) -> Result<(), RenderError> {
    let findings_doc =
        render_typst_in_staging(staging_dir, record_str, cache, http).and_then(|file| {
            Document::load(&file).map_err(|error| RenderError::PdfReadError { file, error })
        })?;

    let mut doc = if !qc_context.is_empty() {
        let context_docs = qc_context
            .iter()
            .map(|context| load_context_file(&context.file).map(|d| (d, context.position)))
            .collect::<Result<Vec<(Document, ContextPosition)>, RenderError>>()?;

        merge_pdfs(findings_doc, context_docs)?
    } else {
        findings_doc
    };

    doc.save(output_path)?;

    Ok(())
}

fn merge_pdfs(
    findings_doc: Document,
    context_docs: Vec<(Document, ContextPosition)>,
) -> Result<Document, RenderError> {
    let mut prepended_docs = Vec::new();
    let mut appended_docs = Vec::new();
    for (doc, pos) in context_docs {
        match pos {
            ContextPosition::Prepend => prepended_docs.push(doc),
            ContextPosition::Append => appended_docs.push(doc),
        }
    }
    let mut docs = prepended_docs;
    docs.push(findings_doc);
    docs.append(&mut appended_docs);

    // Define a starting `max_id` (will be used as start index for object_ids).
    let mut max_id = 1;
    let mut pagenum = 1;
    // Collect all Documents Objects grouped by a map
    let mut documents_pages = BTreeMap::new();
    let mut documents_objects = BTreeMap::new();
    let mut document = Document::with_version("1.5");

    for mut doc in docs {
        let mut first = false;
        doc.renumber_objects_with(max_id);

        max_id = doc.max_id + 1;

        for (_, object_id) in doc.get_pages() {
            if !first {
                let bookmark = Bookmark::new(
                    String::from(format!("Page_{}", pagenum)),
                    [0.0, 0.0, 1.0],
                    0,
                    object_id,
                );
                document.add_bookmark(bookmark, None);
                first = true;
                pagenum += 1;
            }

            let obj = doc
                .get_object(object_id)
                .map_err(|_| RenderError::PdfMergeMissingObject(object_id))?
                .to_owned();
            documents_pages.insert(object_id, obj);
        }
        documents_objects.extend(doc.objects);
    }

    // "Catalog" and "Pages" are mandatory.
    let mut catalog_object: Option<(ObjectId, Object)> = None;
    let mut pages_object: Option<(ObjectId, Object)> = None;

    // Process all objects except "Page" type
    for (object_id, object) in documents_objects.iter() {
        // We have to ignore "Page" (as are processed later), "Outlines" and "Outline" objects.
        // All other objects should be collected and inserted into the main Document.
        match object.type_name().unwrap_or(b"") {
            b"Catalog" => {
                // Collect a first "Catalog" object and use it for the future "Pages".
                catalog_object = Some((
                    if let Some((id, _)) = catalog_object {
                        id
                    } else {
                        *object_id
                    },
                    object.clone(),
                ));
            }
            b"Pages" => {
                // Collect and update a first "Pages" object and use it for the future "Catalog"
                // We have also to merge all dictionaries of the old and the new "Pages" object
                if let Ok(dictionary) = object.as_dict() {
                    let mut dictionary = dictionary.clone();
                    if let Some((_, ref object)) = pages_object {
                        if let Ok(old_dictionary) = object.as_dict() {
                            dictionary.extend(old_dictionary);
                        }
                    }

                    pages_object = Some((
                        if let Some((id, _)) = pages_object {
                            id
                        } else {
                            *object_id
                        },
                        Object::Dictionary(dictionary),
                    ));
                }
            }
            b"Page" => {}     // Ignored, processed later and separately
            b"Outlines" => {} // Ignored, not supported yet
            b"Outline" => {}  // Ignored, not supported yet
            _ => {
                document.objects.insert(*object_id, object.clone());
            }
        }
    }

    // If no "Pages" object found, abort.
    let Some(pages_object) = pages_object else {
        return Err(RenderError::PdfMergeMissingPages);
    };

    // Iterate over all "Page" objects and collect into the parent "Pages" created before
    for (object_id, object) in documents_pages.iter() {
        if let Ok(dictionary) = object.as_dict() {
            let mut dictionary = dictionary.clone();
            dictionary.set("Parent", pages_object.0);

            document
                .objects
                .insert(*object_id, Object::Dictionary(dictionary));
        }
    }

    // If no "Catalog" found, abort.
    let Some(catalog_object) = catalog_object else {
        return Err(RenderError::PdfMergeMissingCatalog);
    };

    // Build a new "Pages" with updated fields
    if let Ok(dictionary) = pages_object.1.as_dict() {
        let mut dictionary = dictionary.clone();

        // Set new pages count
        dictionary.set("Count", documents_pages.len() as u32);

        // Set new "Kids" list (collected from documents pages) for "Pages"
        dictionary.set(
            "Kids",
            documents_pages
                .into_iter()
                .map(|(object_id, _)| Object::Reference(object_id))
                .collect::<Vec<_>>(),
        );

        document
            .objects
            .insert(pages_object.0, Object::Dictionary(dictionary));
    }

    // Build a new "Catalog" with updated fields
    if let Ok(dictionary) = catalog_object.1.as_dict() {
        let mut dictionary = dictionary.clone();
        dictionary.set("Pages", pages_object.0);
        dictionary.remove(b"Outlines"); // Outlines not supported in merged PDFs

        document
            .objects
            .insert(catalog_object.0, Object::Dictionary(dictionary));
    }

    document.trailer.set("Root", catalog_object.0);

    // Update the max internal ID as wasn't updated before due to direct objects insertion
    document.max_id = document.objects.len() as u32;

    // Reorder all new Document objects
    document.renumber_objects();

    // Set any Bookmarks to the First child if they are not set to a page
    document.adjust_zero_pages();

    // Set all bookmarks to the PDF Object tree then set the Outlines to the Bookmark content map.
    if let Some(n) = document.build_outline() {
        if let Ok(Object::Dictionary(dict)) = document.get_object_mut(catalog_object.0) {
            dict.set("Outlines", Object::Reference(n));
        }
    }

    document.compress();

    Ok(document)
}

fn render_typst_in_staging(
    staging_dir: &Path,
    report: &str,
    cache: Option<&DiskCache>,
    http: &(impl HttpDownloader + Clone + 'static),
) -> Result<PathBuf, RenderError> {
    let cache_dir = cache
        .map(|c| c.root.to_path_buf())
        .unwrap_or(tempdir().map_err(RenderError::Io)?.path().to_path_buf());
    let world = TypstWorld::new(staging_dir, report.to_string(), &cache_dir, http.clone());
    log::debug!("Rendering pdf record from typst...");
    let generate_compile_error_message = |v: EcoVec<SourceDiagnostic>| -> RenderError {
        let err = v
            .iter()
            .map(|s| {
                format!(
                    "{}: {}",
                    match s.severity {
                        Severity::Error => "ERROR",
                        Severity::Warning => "WARNING",
                    },
                    s.message
                )
            })
            .collect::<Vec<_>>()
            .join("\n\t");

        RenderError::TypstCompile(err)
    };

    let document = typst::compile(&world)
        .output
        .map_err(generate_compile_error_message)?;

    let pdf = typst_pdf::pdf(&document, &PdfOptions::default())
        .map_err(generate_compile_error_message)?;

    let staging_pdf_path = staging_dir.join("record.pdf");

    fs::write(&staging_pdf_path, pdf).map_err(RenderError::Io)?;

    Ok(staging_pdf_path)
}

/// Load a context file as a PDF Document.
/// Only PDF files are supported - users must convert Word documents to PDF first.
fn load_context_file(file: impl AsRef<Path>) -> Result<Document, RenderError> {
    let input_file = file.as_ref();

    // Check if the file is a PDF
    let is_pdf = input_file
        .extension()
        .map(|ext| ext.to_string_lossy().to_lowercase() == "pdf")
        .unwrap_or(false);

    if !is_pdf {
        return Err(RenderError::UnsupportedFileFormat {
            file: input_file.to_path_buf(),
        });
    }

    log::debug!("Loading PDF: {}", input_file.display());
    Document::load(input_file).map_err(|error| RenderError::PdfReadError {
        file: input_file.to_path_buf(),
        error,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("Typst Compile Failed: {0}")]
    TypstCompile(String),
    #[error("IO Error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Failed to read pdf at {file}: {error}")]
    PdfReadError { file: PathBuf, error: lopdf::Error },
    #[error("Failed to write pdf: {0}")]
    PdfWriteError(#[from] lopdf::Error),
    #[error("PDF merge failed: Pages root not found in source documents")]
    PdfMergeMissingPages,
    #[error("PDF merge failed: Catalog not found in source documents")]
    PdfMergeMissingCatalog,
    #[error("PDF merge failed: Could not retrieve object {0:?}")]
    PdfMergeMissingObject(ObjectId),
    #[error(
        "Unsupported file format for {file}: only PDF files are supported. Please convert the file to PDF first."
    )]
    UnsupportedFileFormat { file: PathBuf },
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // Helper to create a minimal valid PDF for testing
    fn create_test_pdf() -> Document {
        use lopdf::dictionary;

        let mut doc = Document::with_version("1.5");

        // Create a minimal page
        let pages_id = doc.new_object_id();
        let page_id = doc.new_object_id();
        let content_id = doc.new_object_id();

        // Add content stream (empty)
        doc.objects.insert(
            content_id,
            Object::Stream(lopdf::Stream::new(dictionary! {}, vec![])),
        );

        // Add page
        let page = dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        };
        doc.objects.insert(page_id, Object::Dictionary(page));

        // Add pages
        let pages = dictionary! {
            "Type" => "Pages",
            "Kids" => vec![Object::Reference(page_id)],
            "Count" => 1,
        };
        doc.objects.insert(pages_id, Object::Dictionary(pages));

        // Add catalog
        let catalog_id = doc.new_object_id();
        let catalog = dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        };
        doc.objects.insert(catalog_id, Object::Dictionary(catalog));

        // Set trailer
        doc.trailer.set("Root", catalog_id);
        doc.max_id = doc.objects.len() as u32;

        doc
    }

    // ===================
    // QCContext tests
    // ===================

    #[test]
    fn test_qc_context_new() {
        let ctx = QCContext::new("/path/to/file.pdf", ContextPosition::Prepend);
        assert_eq!(ctx.file(), Path::new("/path/to/file.pdf"));
        assert!(matches!(ctx.position(), ContextPosition::Prepend));
    }

    #[test]
    fn test_qc_context_append_position() {
        let ctx = QCContext::new("document.docx", ContextPosition::Append);
        assert_eq!(ctx.file(), Path::new("document.docx"));
        assert!(matches!(ctx.position(), ContextPosition::Append));
    }

    #[test]
    fn test_qc_context_clone() {
        let ctx1 = QCContext::new("/test/path.pdf", ContextPosition::Prepend);
        let ctx2 = ctx1.clone();
        assert_eq!(ctx1.file(), ctx2.file());
        assert!(matches!(ctx1.position(), ContextPosition::Prepend));
        assert!(matches!(ctx2.position(), ContextPosition::Prepend));
    }

    // ===================
    // load_context_file tests
    // ===================

    #[test]
    fn test_load_context_file_pdf() {
        let temp_dir = TempDir::new().unwrap();
        let pdf_path = temp_dir.path().join("test.pdf");

        // Create and save a test PDF
        let mut doc = create_test_pdf();
        doc.save(&pdf_path).unwrap();

        let result = load_context_file(&pdf_path);
        assert!(result.is_ok());
    }

    #[test]
    fn test_load_context_file_non_pdf_rejected() {
        let temp_dir = TempDir::new().unwrap();
        let docx_path = temp_dir.path().join("test.docx");

        // Create a dummy file
        std::fs::write(&docx_path, b"dummy content").unwrap();

        let result = load_context_file(&docx_path);

        assert!(result.is_err());
        match result.unwrap_err() {
            RenderError::UnsupportedFileFormat { file } => {
                assert_eq!(file, docx_path);
            }
            e => panic!("Expected UnsupportedFileFormat, got {:?}", e),
        }
    }

    #[test]
    fn test_load_context_file_pdf_case_insensitive() {
        let temp_dir = TempDir::new().unwrap();
        let pdf_path = temp_dir.path().join("test.PDF"); // uppercase extension

        // Create and save a test PDF
        let mut doc = create_test_pdf();
        doc.save(&pdf_path).unwrap();

        // Should recognize .PDF as PDF
        let result = load_context_file(&pdf_path);
        assert!(result.is_ok());
    }

    // ===================
    // merge_pdfs tests
    // ===================

    #[test]
    fn test_merge_pdfs_no_context() {
        let findings = create_test_pdf();
        let context_docs = vec![];

        let result = merge_pdfs(findings, context_docs);
        assert!(result.is_ok());

        let merged = result.unwrap();
        assert_eq!(merged.get_pages().len(), 1);
    }

    #[test]
    fn test_merge_pdfs_with_prepend() {
        let findings = create_test_pdf();
        let prepend_doc = create_test_pdf();

        let context_docs = vec![(prepend_doc, ContextPosition::Prepend)];

        let result = merge_pdfs(findings, context_docs);
        assert!(result.is_ok());

        let merged = result.unwrap();
        assert_eq!(merged.get_pages().len(), 2);
    }

    #[test]
    fn test_merge_pdfs_with_append() {
        let findings = create_test_pdf();
        let append_doc = create_test_pdf();

        let context_docs = vec![(append_doc, ContextPosition::Append)];

        let result = merge_pdfs(findings, context_docs);
        assert!(result.is_ok());

        let merged = result.unwrap();
        assert_eq!(merged.get_pages().len(), 2);
    }

    #[test]
    fn test_merge_pdfs_prepend_and_append() {
        let findings = create_test_pdf();
        let prepend_doc = create_test_pdf();
        let append_doc = create_test_pdf();

        let context_docs = vec![
            (prepend_doc, ContextPosition::Prepend),
            (append_doc, ContextPosition::Append),
        ];

        let result = merge_pdfs(findings, context_docs);
        assert!(result.is_ok());

        let merged = result.unwrap();
        assert_eq!(merged.get_pages().len(), 3);
    }

    #[test]
    fn test_merge_pdfs_multiple_prepend() {
        let findings = create_test_pdf();
        let prepend1 = create_test_pdf();
        let prepend2 = create_test_pdf();

        let context_docs = vec![
            (prepend1, ContextPosition::Prepend),
            (prepend2, ContextPosition::Prepend),
        ];

        let result = merge_pdfs(findings, context_docs);
        assert!(result.is_ok());

        let merged = result.unwrap();
        assert_eq!(merged.get_pages().len(), 3);
    }

    #[test]
    fn test_merge_pdfs_output_is_valid() {
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("merged.pdf");

        let findings = create_test_pdf();
        let prepend_doc = create_test_pdf();

        let context_docs = vec![(prepend_doc, ContextPosition::Prepend)];

        let mut merged = merge_pdfs(findings, context_docs).unwrap();

        // Save and reload to verify it's a valid PDF
        merged.save(&output_path).unwrap();
        let reloaded = Document::load(&output_path);
        assert!(reloaded.is_ok());
    }
}
