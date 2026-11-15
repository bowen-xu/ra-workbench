import type { LiteratureCreator, LiteratureDraft, LiteratureItem } from "@research-assistant/core/literature";

interface PreviewLiteratureFixture {
  readonly pdfUrl: string;
  readonly item: LiteratureItem;
}

function author(givenName: string, familyName: string): LiteratureCreator {
  return { givenName, familyName, literalName: "", role: "author" };
}

function fixture(draft: LiteratureDraft & { readonly id: string }, attachment: {
  readonly id: string; readonly fileName: string; readonly byteSize: number; readonly sha256: string;
}): PreviewLiteratureFixture {
  const timestamp = "2026-10-09T00:00:00.000Z";
  return {
    pdfUrl: `/preview/${attachment.fileName}`,
    item: { ...draft, readingStatus: "unread", projectIds: [],
      attachments: [{ id: attachment.id, displayName: attachment.fileName, mimeType: "application/pdf",
        byteSize: attachment.byteSize, contentSha256: attachment.sha256 }],
      metadataSources: ["title", "creators", "issuedYear", "doi", "containerTitle"].map(fieldName =>
        ({ fieldName, sourceKind: "pdf", confirmedByUser: false })),
      createdAt: timestamp, updatedAt: timestamp },
  };
}

// Bibliographic fields come from the supplied PDFs. Missing publication fields
// (including the interoception paper's final volume/pages) remain unspecified.
export const previewLiteratureFixtures: readonly PreviewLiteratureFixture[] = [
  fixture({
    id: "10000000-0000-4000-8000-000000000004", itemType: "conferencePaper",
    title: "Vision by Logic: A Naïve Visual Recognition Model with Non-axiomatic Logic",
    creators: [author("Bowen", "Xu"), author("Boyang", "Xu"), author("Pei", "Wang")],
    issuedYear: 2026, containerTitle: "Artificial General Intelligence", conferenceName: "AGI 2026",
    proceedingsTitle: "Artificial General Intelligence", volume: "16855", issue: "", pages: "440–454",
    doi: "10.1007/978-3-032-33195-3_30", url: "https://doi.org/10.1007/978-3-032-33195-3_30",
    language: "en", bibtexKey: "xu2026vision", sourceKind: "pdf",
    abstractText: "Logic-based systems have traditionally been considered unsuitable for perceptual tasks. In this work, we explore an alternative approach in which visual recognition is treated as a process of logical inference. Building on Non-Axiomatic Logic (NAL), a logic for reasoning under uncertainty, we represent objects as compositional concepts defined by parts and their spatial relations, while local observations provide graded evidential support for competing hypotheses. Recognition is thus formulated as an incremental process of evidence accumulation and belief revision, unifying perceptual processing and logical reasoning within a single framework. To demonstrate the feasibility of this perspective, we construct a minimal model, the Convolutional Concept Network (CCN), which combines convolution-like operations with NAL-based inference. The model maintains explicit, interpretable intermediate representations. Experiments on MNIST show that the system learns hierarchical concepts, suggesting that logic can be applied to visual recognition effectively.",
    tags: ["预览样本", "视觉识别", "非公理逻辑"],
  }, { id: "10000000-0000-4000-8000-000000000005", fileName: "vision-by-logic.pdf", byteSize: 823475,
    sha256: "39a0b674e2cdc8297e7e92c6c0b879bf07ea65f7de8c116db002f725cb8289cd" }),
  fixture({
    id: "10000000-0000-4000-8000-000000000006", itemType: "journalArticle",
    title: "Automated construction of cognitive maps with visual predictive coding",
    creators: [author("James", "Gornet"), author("Matt", "Thomson")],
    issuedYear: 2024, containerTitle: "Nature Machine Intelligence", volume: "6", issue: "", pages: "820–833",
    doi: "10.1038/s42256-024-00863-1", url: "https://doi.org/10.1038/s42256-024-00863-1",
    language: "en", bibtexKey: "gornet2024cognitive", sourceKind: "pdf",
    abstractText: "Humans construct internal cognitive maps of their environment directly from sensory inputs without access to a system of explicit coordinates or distance measurements. Although machine learning algorithms like simultaneous localization and mapping utilize specialized inference procedures to identify visual features and construct spatial maps from visual and odometry data, the general nature of cognitive maps in the brain suggests a unified mapping algorithmic strategy that can generalize to auditory, tactile and linguistic inputs. Here we demonstrate that predictive coding provides a natural and versatile neural network algorithm for constructing spatial maps using sensory data. We introduce a framework in which an agent navigates a virtual environment while engaging in visual predictive coding using a self-attention-equipped convolutional neural network. While learning a next-image prediction task, the agent automatically constructs an internal representation of the environment that quantitatively reflects spatial distances. The internal map enables the agent to pinpoint its location relative to landmarks using only visual information. The predictive coding network generates a vectorized encoding of the environment that supports vector navigation, where individual latent space units delineate localized, overlapping neighbourhoods in the environment. Broadly, our work introduces predictive coding as a unified algorithmic framework for constructing cognitive maps that can naturally extend to the mapping of auditory, sensorimotor and linguistic inputs.",
    tags: ["预览样本", "认知地图", "预测编码"],
  }, { id: "10000000-0000-4000-8000-000000000007", fileName: "cognitive-maps.pdf", byteSize: 8098027,
    sha256: "c51c720d0f38406f9e4144443f938866d0fe7b096f2331515cfa01f9e4da7cfe" }),
  fixture({
    id: "10000000-0000-4000-8000-000000000008", itemType: "journalArticle",
    title: "Life-inspired interoceptive artificial intelligence for autonomous and adaptive agents",
    creators: [author("Sungwoo", "Lee"), author("Younghyun", "Oh"), author("Hyunhoe", "An"),
      author("Hyebhin", "Yoon"), author("Karl J.", "Friston"), author("Seok Jun", "Hong"), author("Choong-Wan", "Woo")],
    issuedYear: 2026, containerTitle: "Nature Machine Intelligence", volume: "", issue: "", pages: "",
    doi: "10.1038/s42256-026-01296-8", url: "https://doi.org/10.1038/s42256-026-01296-8",
    language: "en", bibtexKey: "lee2026interoceptive", sourceKind: "pdf",
    abstractText: "Despite remarkable advances in artificial intelligence (AI), building agents that can autonomously pursue goals while adapting to continuously changing environments remains a fundamental challenge. Living organisms naturally exhibit such adaptive autonomy, offering a compelling source of inspiration for how artificial systems might achieve similar capabilities. Here we focus on interoception—the process of monitoring and regulating internal bodily states to maintain internal homeostasis—which underwrites an organism’s survival. Developing interoceptive AI requires abstracting internal states from their biological instantiation into functional and mathematical representations that are applicable to artificial systems. This requires explicit factorization of state variables representing internal and external environments, together with mathematical formalization of life-inspired properties governing internal-state dynamics. Importantly, internal states can also function as universally available and intrinsically valuable contexts, serving as stable reference signals that modulate learning and behaviour under changing external environments. In addition, interoception-related biological processes, such as neuromodulatory mechanisms, can be incorporated to support context-dependent adaptive behaviour. Overall, this Perspective presents a unified account on how interoception can enhance autonomy and adaptivity in artificial agents by integrating insights from cybernetics with recent advances in theories of life, reinforcement learning and neuroscience.",
    tags: ["预览样本", "内感受", "自主智能体"],
  }, { id: "10000000-0000-4000-8000-000000000009", fileName: "interoceptive-ai.pdf", byteSize: 1983518,
    sha256: "b53ca8a3946d94e5b48be975e7f4a5a852103daab68f6653495508cdfa074f1f" }),
];

/** Add a new fixture set once, preserving edits and deletions on later starts. */
export function seedPreviewLiterature(storage: Storage): void {
  const marker = "literature-fixtures-v1";
  if (storage.getItem(marker) === "1") return;
  const key = "research-assistant.literature.v1";
  const items: LiteratureItem[] = JSON.parse(storage.getItem(key) ?? "[]") as LiteratureItem[];
  const trash: LiteratureItem[] = JSON.parse(storage.getItem("research-assistant.literature-trash.v1") ?? "[]") as LiteratureItem[];
  const existing = new Set([...items, ...trash].map(item => item.id));
  storage.setItem(key, JSON.stringify([...items, ...previewLiteratureFixtures.filter(value => !existing.has(value.item.id)).map(value => value.item)]));
  storage.setItem(marker, "1");
}
