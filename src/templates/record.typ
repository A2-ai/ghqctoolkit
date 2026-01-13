// Document settings
#set document(
  title: "QC Record: {{ milestone_names }}",
  {% if author %}author: "{{ author }}",{% endif %}
  date: auto,
)

#set page(
  paper: "us-letter",
  margin: (x: 1in, y: 1in),
  {% if logo_path %}
  header: align(right)[#image("{{ logo_path }}", height: 0.7in)],
  header-ascent: 25%,
  {% endif %}
  footer: context [
    #align(center)[Page #counter(page).display() of #counter(page).final().first()]
  ],
)

#set text(
  font: "Times New Roman",
  size: 12pt,
)

#set heading(numbering: none)

// Title page
#align(center)[
  #text(size: 24pt, weight: "bold")[QC Record: {{ milestone_names }}]

  #v(1em)

  #text(size: 14pt)[Git repository: {{ repository_name }}]

  #v(1em)

  {% if author %}#text(size: 12pt)[Author: {{ author }}]

  {% endif %}
  #text(size: 12pt)[Date: {{ date }}]
]

#v(2em)

#outline(title: "Table of Contents", depth: 2)

#pagebreak()

= Milestone Summary

#table(
  columns: (0.20fr, 0.20fr, auto, 0.43fr),
  stroke: none,
  inset: 8pt,
  align: (left, left, left, left),
  table.hline(),
  table.header(
    [*Title*], [*Description*], [*Status*], [*Issues*],
  ),
  table.hline(),
  {{ render_milestone_table_rows(data=milestone_data) }}
  table.hline(),
)

#v(1em)
#text(fill: red)[U] Unapproved Issue \
#text(fill: red)[C] Issue with unchecked items

{% for section in milestone_sections %}
#pagebreak()

= {{ section.name }}

== Issue Summary

#table(
  columns: (1fr, 1fr, 1fr, 1fr, 1fr),
  stroke: none,
  inset: 8pt,
  align: (left, left, left, left, left),
  table.hline(),
  table.header(
    [*File Path*], [*QC Status*], [*Author*], [*QCer*], [*Issue Closer*],
  ),
  table.hline(),
  {{ render_issue_summary_table_rows(data=section.issues) }}
  table.hline(),
)

#pagebreak()

{% if not only_tables %}
{% for issue in section.issues %}
== {{ issue.title }}

=== Issue Information

- *Issue Number:* {{ issue.number }}
- *Milestone:* `{{ issue.milestone }}`
- *Created by:* {{ issue.created_by }}
- *Created at:* {{ issue.created_at }}
- *QCer:* {{ issue.qcer | join(sep=", ") }}
- *QC Status:* {{ issue.qc_status }}
- *{{ checklist_name | title }} Summary:* {{ issue.checklist_summary }}
- *Git Status:* {{ issue.git_status }}
- *Initial QC Commit:* {{ issue.initial_qc_commit }}
- *Latest QC Commit:* {{ issue.latest_qc_commit }}
- *Issue URL:* {{ issue.issue_url }}
- *Issue state:* {{ issue.state }}
{% if issue.closed_by %}
- *Closed by:* {{ issue.closed_by }}
- *Closed at:* {{ issue.closed_at }}
{% endif %}

=== Issue Body

{{ issue.body }}

=== Comments

{% if issue.comments %}
{% for comment in issue.comments %}
==== {{ comment.0 }}

{{ comment.1 }}

{% endfor %}
{% else %}
No comments found.
{% endif %}

=== Events

{% if issue.events %}
{% for event in issue.events %}
- {{ event }}
{% endfor %}
{% else %}
No events found.
{% endif %}

=== Detailed Timeline

{% if issue.timeline %}
{% for item in issue.timeline %}
- {{ item }}
{% endfor %}
{% else %}
No timeline items found.
{% endif %}

#pagebreak()
{% endfor %}
{% endif %}
{% endfor %}
