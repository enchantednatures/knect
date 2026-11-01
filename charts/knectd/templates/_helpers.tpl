{{/*
Chart name, overridable but always DNS-safe.
*/}}
{{- define "knectd.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{/*
Fully qualified release name.
*/}}
{{- define "knectd.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := default .Chart.Name .Values.nameOverride -}}
{{- if contains $name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{/*
Chart label, safe for label values.
*/}}
{{- define "knectd.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{/*
Common metadata labels.
*/}}
{{- define "knectd.labels" -}}
helm.sh/chart: {{ include "knectd.chart" . }}
{{ include "knectd.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: knect
{{- end -}}

{{/*
Immutable selector labels.
*/}}
{{- define "knectd.selectorLabels" -}}
app.kubernetes.io/name: {{ include "knectd.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{/*
Service account name.
*/}}
{{- define "knectd.serviceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
{{- default (include "knectd.fullname" .) .Values.serviceAccount.name -}}
{{- else -}}
{{- default "default" .Values.serviceAccount.name -}}
{{- end -}}
{{- end -}}

{{/*
Observer ClusterRole name.
*/}}
{{- define "knectd.clusterRoleName" -}}
{{- default (include "knectd.fullname" .) .Values.rbac.clusterRoleName -}}
{{- end -}}

{{/*
Actions Role name.
*/}}
{{- define "knectd.actionsRoleName" -}}
{{- default (printf "%s-actions" (include "knectd.fullname" .)) .Values.actions.role.name -}}
{{- end -}}

{{/*
Image reference, honoring digest over tag.
*/}}
{{- define "knectd.image" -}}
{{- if .Values.image.digest -}}
{{- printf "%s@%s" .Values.image.repository .Values.image.digest -}}
{{- else -}}
{{- printf "%s:%s" .Values.image.repository (default .Chart.AppVersion .Values.image.tag) -}}
{{- end -}}
{{- end -}}
