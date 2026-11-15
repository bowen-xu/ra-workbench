/** Synthetic model output for exercising the production Markdown renderer. */
export const writingDemoListMathExample = String.raw`1. **列表公式**：例如在
   \[
   a+b=c
   \]
   中，行内 \(C_i\) 应与前后文字处于同一行。
2. **下一项**：列表继续正常排版。`;

const wideSum = Array.from({length: 12}, (_, index) => String.raw`\sum_{i=1}^{n}\frac{(x_{i,${index + 1}}-\mu_{${index + 1}})^2}{\sigma_{${index + 1}}^2}`).join("+");
export const writingDemoMarkdownMessages = [
  "# 稿件检查\n\n这是**模拟反馈**，包含 *强调*、~~删除线~~ 和 `keywords`。\n\n- 检查关键词\n- 补充摘要\n\n> 下列内容均为演示数据。",
  "## 修改对照\n\n| 项目 | 修改前 | 修改后 |\n| :--- | :---: | ---: |\n| 关键词数量 | 2 | 3 |\n| 摘要段落 | 0 | 1 |\n\n表格中的数字仅用于模拟。",
  String.raw`## 公式说明

行内公式：\(E = mc^2\)。

${writingDemoListMathExample}

均值与方差：

\[
\bar{x} = \frac{1}{n}\sum_{i=1}^{n}x_i,\qquad
s^2 = \frac{1}{n-1}\sum_{i=1}^{n}(x_i-\bar{x})^2
\]

较高的行内公式：\(\dfrac{1+\dfrac{a}{b}}{1+\dfrac{c}{d}}\)，应自然撑开所在行，不遮挡前后文字。

矩阵：

\[
A=\begin{pmatrix}
a_{11}&a_{12}&a_{13}&a_{14}\\
a_{21}&a_{22}&a_{23}&a_{24}\\
a_{31}&a_{32}&a_{33}&a_{34}\\
a_{41}&a_{42}&a_{43}&a_{44}
\end{pmatrix}
\]

超宽行内公式：\(\displaystyle L=${wideSum}\)。后面的文字仍保留。

超宽独立公式：

\[
\mathcal{L}=${wideSum}
\]`,
  "### 检查步骤\n\n1. 读取稿件\n2. 核对修改\n\n- [x] 关键词已更新\n- [x] 摘要已补充\n\n```python\nvalues = [1, 2, 3]\nmean = sum(values) / len(values)\nprint(mean)\n```\n\n[Markdown 格式说明](https://commonmark.org/help/)\n\n---\n\n**模拟检查完成。**",
] as const;
