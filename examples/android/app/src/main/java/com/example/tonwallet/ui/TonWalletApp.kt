package com.example.tonwallet.ui

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.view.WindowManager
import androidx.activity.compose.BackHandler
import androidx.activity.compose.LocalActivity
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.calculateEndPadding
import androidx.compose.foundation.layout.calculateStartPadding
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.sizeIn
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.automirrored.rounded.Send
import androidx.compose.material.icons.rounded.AccountBalanceWallet
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.ArrowDownward
import androidx.compose.material.icons.rounded.ArrowUpward
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.CheckCircle
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.ContentCopy
import androidx.compose.material.icons.rounded.DeleteOutline
import androidx.compose.material.icons.rounded.Edit
import androidx.compose.material.icons.rounded.KeyboardArrowDown
import androidx.compose.material.icons.rounded.KeyboardArrowUp
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.MoreHoriz
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Security
import androidx.compose.material.icons.rounded.Shield
import androidx.compose.material.icons.rounded.SouthWest
import androidx.compose.material.icons.rounded.Warning
import androidx.compose.material.icons.rounded.Visibility
import androidx.compose.material.icons.rounded.VisibilityOff
import androidx.compose.material.icons.rounded.Wifi
import androidx.compose.material.icons.rounded.WifiOff
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import com.example.tonwallet.WalletUiState
import com.example.tonwallet.WalletViewModel
import com.example.tonwallet.data.AccountSnapshot
import com.example.tonwallet.data.StoredWallet
import com.example.tonwallet.data.WalletTransaction
import com.example.tonwallet.ui.theme.GramBlue
import com.example.tonwallet.ui.theme.LocalAppSpacing
import com.example.tonwallet.ui.theme.Success
import com.example.tonwallet.ui.theme.Warning
import java.text.DateFormat
import java.util.Date
import java.util.Locale

private enum class WalletSheet { Create, Import, Send, Receive, Settings }

private const val UI_PREFERENCES = "wallet_ui_preferences"
private const val BALANCE_VISIBLE_KEY = "is_balance_visible"

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TonWalletApp(viewModel: WalletViewModel, modifier: Modifier = Modifier) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val context = LocalContext.current
    val uiPreferences = remember(context) {
        context.getSharedPreferences(UI_PREFERENCES, Context.MODE_PRIVATE)
    }
    val snackbarHostState = remember { SnackbarHostState() }
    var sheet by rememberSaveable { mutableStateOf<WalletSheet?>(null) }
    var walletMenuOpen by remember { mutableStateOf(false) }
    var selectedTransaction by remember { mutableStateOf<WalletTransaction?>(null) }
    var isBalanceVisible by rememberSaveable {
        mutableStateOf(uiPreferences.getBoolean(BALANCE_VISIBLE_KEY, true))
    }
    AutoRefreshEffect(viewModel)

    BackHandler(enabled = selectedTransaction != null) {
        selectedTransaction = null
    }
    LaunchedEffect(state.activeWallet?.address) {
        selectedTransaction = null
    }

    LaunchedEffect(state.error, state.notice) {
        val message = state.error ?: state.notice ?: return@LaunchedEffect
        if (sheet == WalletSheet.Send && state.error != null) return@LaunchedEffect
        if (state.notice == "Transfer submitted to testnet") sheet = null
        snackbarHostState.showSnackbar(message)
        viewModel.consumeMessages()
    }

    Scaffold(
        modifier = modifier.fillMaxSize(),
        snackbarHost = { SnackbarHost(snackbarHostState) },
        topBar = {
            if (selectedTransaction != null) {
                CenterAlignedTopAppBar(
                    title = { Text("Transaction") },
                    colors = TopAppBarDefaults.topAppBarColors(
                        containerColor = MaterialTheme.colorScheme.background,
                    ),
                    navigationIcon = {
                        IconButton(onClick = { selectedTransaction = null }) {
                            Icon(Icons.AutoMirrored.Rounded.ArrowBack, contentDescription = "Back")
                        }
                    },
                )
            } else if (state.activeWallet != null) {
                CenterAlignedTopAppBar(
                    title = {
                        Box {
                            TextButton(onClick = { walletMenuOpen = true }) {
                                Text(
                                    state.activeWallet?.name.orEmpty(),
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis,
                                )
                                Icon(Icons.Rounded.KeyboardArrowDown, contentDescription = null)
                            }
                            DropdownMenu(
                                expanded = walletMenuOpen,
                                onDismissRequest = { walletMenuOpen = false },
                            ) {
                                state.wallets.forEach { wallet ->
                                    DropdownMenuItem(
                                        text = { Text(wallet.name) },
                                        leadingIcon = if (wallet.address == state.activeWallet?.address) {
                                            { Icon(Icons.Rounded.Check, contentDescription = null) }
                                        } else null,
                                        onClick = {
                                            walletMenuOpen = false
                                            viewModel.selectWallet(wallet.address)
                                        },
                                    )
                                }
                            }
                        }
                    },
                    navigationIcon = {
                        IconButton(onClick = { sheet = WalletSheet.Settings }) {
                            Icon(Icons.Rounded.MoreHoriz, contentDescription = "Wallet settings")
                        }
                    },
                    actions = {
                        IconButton(onClick = { sheet = WalletSheet.Create }) {
                            Icon(Icons.Rounded.Add, contentDescription = "Create wallet")
                        }
                    },
                )
            }
        },
    ) { innerPadding ->
        val transaction = selectedTransaction
        if (transaction != null) {
            TransactionDetailScreen(
                transaction = transaction,
                usdPerTon = state.usdPerTon,
                contentPadding = innerPadding,
            )
        } else {
            AnimatedContent(
                targetState = state.activeWallet == null,
                label = "wallet-content",
            ) { isEmpty ->
                if (isEmpty) {
                WelcomeScreen(
                    contentPadding = innerPadding,
                    onCreate = { sheet = WalletSheet.Create },
                    onImport = { sheet = WalletSheet.Import },
                )
                } else {
                    DashboardScreen(
                        state = state,
                        contentPadding = innerPadding,
                        isBalanceVisible = isBalanceVisible,
                        onToggleBalanceVisibility = {
                            isBalanceVisible = !isBalanceVisible
                            uiPreferences.edit()
                                .putBoolean(BALANCE_VISIBLE_KEY, isBalanceVisible)
                                .apply()
                        },
                        onRefresh = viewModel::refresh,
                        onSend = {
                            viewModel.clearSendError()
                            sheet = WalletSheet.Send
                        },
                        onReceive = { sheet = WalletSheet.Receive },
                        onLoadMore = viewModel::loadMore,
                        onTransactionClick = { selectedTransaction = it },
                    )
                }
            }
        }
    }

    when (sheet) {
        WalletSheet.Create -> CreateWalletSheet(
            onDismiss = { sheet = null },
            onCreate = { name ->
                sheet = null
                viewModel.createWallet(name)
            },
        )
        WalletSheet.Import -> ImportWalletSheet(
            onDismiss = { sheet = null },
            onImport = viewModel::importWallet,
        )
        WalletSheet.Send -> SendSheet(
            state = state,
            onDismiss = {
                viewModel.clearSendError()
                sheet = null
            },
            onSend = viewModel::send,
            onInputChanged = viewModel::clearSendError,
        )
        WalletSheet.Receive -> state.activeWallet?.let {
            ReceiveSheet(it, onDismiss = { sheet = null })
        }
        WalletSheet.Settings -> SettingsSheet(
            state = state,
            onDismiss = { sheet = null },
            onRename = viewModel::renameWallet,
            onDelete = {
                viewModel.deleteWallet()
                sheet = null
            },
            onImport = { sheet = WalletSheet.Import },
        )
        null -> Unit
    }

    state.newRecoveryPhrase?.let { phrase ->
        RecoveryPhraseSheet(
            wallet = state.activeWallet,
            phrase = phrase,
            onDone = {
                viewModel.clearRecoveryPhrase()
                sheet = null
            },
        )
    }
}

@Composable
private fun WelcomeScreen(
    contentPadding: PaddingValues,
    onCreate: () -> Unit,
    onImport: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val spacing = LocalAppSpacing.current
    Column(
        modifier = modifier
            .fillMaxSize()
            .padding(contentPadding)
            .consumeWindowInsets(contentPadding)
            .padding(horizontal = spacing.lg),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Surface(
            modifier = Modifier.size(88.dp),
            shape = MaterialTheme.shapes.large,
            color = MaterialTheme.colorScheme.primaryContainer,
        ) {
            Icon(
                Icons.Rounded.AccountBalanceWallet,
                contentDescription = null,
                modifier = Modifier.padding(spacing.lg),
                tint = MaterialTheme.colorScheme.primary,
            )
        }
        Spacer(Modifier.height(spacing.lg))
        Text(
            "Your TON, in your hands",
            style = MaterialTheme.typography.headlineMedium,
            fontWeight = FontWeight.Bold,
            textAlign = TextAlign.Center,
            modifier = Modifier.semantics { heading() },
        )
        Spacer(Modifier.height(spacing.sm))
        Text(
            "A self-custody wallet for TON testnet. Keys are created on this device and protected by Android Keystore.",
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(spacing.xl))
        Button(
            onClick = onCreate,
            modifier = Modifier.fillMaxWidth().height(56.dp),
        ) {
            Text("Create a new wallet")
        }
        Spacer(Modifier.height(spacing.sm))
        OutlinedButton(
            onClick = onImport,
            modifier = Modifier.fillMaxWidth().height(56.dp),
        ) {
            Text("Import recovery phrase")
        }
        Spacer(Modifier.height(spacing.lg))
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(
                Icons.Rounded.Security,
                contentDescription = null,
                tint = Success,
                modifier = Modifier.size(18.dp),
            )
            Spacer(Modifier.width(spacing.sm))
            Text("Powered by the Rust wallet core", style = MaterialTheme.typography.labelMedium)
        }
    }
}

@Composable
private fun DashboardScreen(
    state: WalletUiState,
    contentPadding: PaddingValues,
    isBalanceVisible: Boolean,
    onToggleBalanceVisibility: () -> Unit,
    onRefresh: () -> Unit,
    onSend: () -> Unit,
    onReceive: () -> Unit,
    onLoadMore: () -> Unit,
    onTransactionClick: (WalletTransaction) -> Unit,
    modifier: Modifier = Modifier,
) {
    val spacing = LocalAppSpacing.current
    val layoutDirection = LocalLayoutDirection.current
    val listPadding = PaddingValues(
        start = contentPadding.calculateStartPadding(layoutDirection) + spacing.md,
        end = contentPadding.calculateEndPadding(layoutDirection) + spacing.md,
        top = contentPadding.calculateTopPadding(),
        bottom = contentPadding.calculateBottomPadding() + spacing.xl,
    )
    LazyColumn(
        modifier = modifier.fillMaxSize(),
        contentPadding = listPadding,
        verticalArrangement = Arrangement.spacedBy(spacing.md),
    ) {
        item(key = "balance") {
            BalanceCard(
                account = state.account,
                usdPerTon = state.usdPerTon,
                isRefreshing = state.isRefreshing,
                isBalanceVisible = isBalanceVisible,
                onToggleBalanceVisibility = onToggleBalanceVisibility,
                onRefresh = onRefresh,
            )
        }
        item(key = "actions") {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(spacing.sm),
            ) {
                WalletAction(
                    label = "Send",
                    icon = Icons.Rounded.ArrowUpward,
                    onClick = onSend,
                    modifier = Modifier.weight(1f),
                )
                WalletAction(
                    label = "Receive",
                    icon = Icons.Rounded.ArrowDownward,
                    onClick = onReceive,
                    modifier = Modifier.weight(1f),
                )
            }
        }
        item(key = "activity-title") {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    "Recent activity",
                    style = MaterialTheme.typography.titleLarge,
                    fontWeight = FontWeight.SemiBold,
                    modifier = Modifier.weight(1f).semantics { heading() },
                )
            }
        }
        if (state.isRefreshing && state.transactions.isEmpty()) {
            item(key = "loading") {
                Box(Modifier.fillMaxWidth().height(120.dp), contentAlignment = Alignment.Center) {
                    CircularProgressIndicator()
                }
            }
        } else if (state.transactions.isEmpty()) {
            item(key = "empty") { EmptyActivity() }
        } else {
            items(state.transactions, key = { it.id }, contentType = { "transaction" }) { transaction ->
                TransactionRow(
                    transaction = transaction,
                    usdPerTon = state.usdPerTon,
                    onClick = { onTransactionClick(transaction) },
                )
            }
        }
        if (state.canLoadMore) {
            item(key = "load-more") {
                OutlinedButton(
                    onClick = onLoadMore,
                    enabled = !state.isLoadingMore,
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    if (state.isLoadingMore) {
                        CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                        Spacer(Modifier.width(spacing.sm))
                    }
                    Text("Load more")
                }
            }
        }
    }
}

@Composable
private fun AutoRefreshEffect(viewModel: WalletViewModel) {
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    DisposableEffect(lifecycle, viewModel) {
        val observer = LifecycleEventObserver { _, event ->
            when (event) {
                Lifecycle.Event.ON_START -> viewModel.startAutoRefresh()
                Lifecycle.Event.ON_STOP -> viewModel.stopAutoRefresh()
                else -> Unit
            }
        }
        lifecycle.addObserver(observer)
        if (lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)) {
            viewModel.startAutoRefresh()
        }
        onDispose {
            lifecycle.removeObserver(observer)
            viewModel.stopAutoRefresh()
        }
    }
}

@Composable
private fun BalanceCard(
    account: AccountSnapshot?,
    usdPerTon: Double?,
    isRefreshing: Boolean,
    isBalanceVisible: Boolean,
    onToggleBalanceVisibility: () -> Unit,
    onRefresh: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val spacing = LocalAppSpacing.current
    val balance = account?.balanceGrams ?: "—"
    val usd = account?.balanceGrams?.toDoubleOrNull()?.let { ton ->
        usdPerTon?.let { rate -> ton * rate }
    }
    Card(
        modifier = modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = Color.Transparent),
        shape = MaterialTheme.shapes.large,
    ) {
        Box(
            modifier = Modifier
                .background(
                    Brush.linearGradient(
                        listOf(GramBlue, Color(0xFF1455D9), Color(0xFF2639A8)),
                    ),
                )
                .fillMaxWidth()
                .padding(spacing.lg),
        ) {
            Column {
                Text(
                    "BALANCE",
                    style = MaterialTheme.typography.labelMedium,
                    color = Color.White.copy(alpha = 0.74f),
                    modifier = Modifier.padding(end = 96.dp),
                )
                Spacer(Modifier.height(spacing.md))
                Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    Text(
                        if (isBalanceVisible) balance else "••••••",
                        fontSize = 42.sp,
                        fontWeight = FontWeight.SemiBold,
                        color = Color.White,
                        maxLines = 1,
                        modifier = Modifier.alignByBaseline(),
                    )
                    Text(
                        "GRAM",
                        style = MaterialTheme.typography.titleLarge,
                        fontWeight = FontWeight.Medium,
                        color = Color.White.copy(alpha = 0.72f),
                        modifier = Modifier.alignByBaseline(),
                    )
                }
                Spacer(Modifier.height(spacing.xs))
                Text(
                    if (isBalanceVisible) {
                        usd?.let { "≈ ${'$'}${String.format(Locale.US, "%,.2f", it)}" }
                            ?: account?.status?.replaceFirstChar { it.uppercase() }
                            ?: "Connecting to testnet…"
                    } else {
                        "••••"
                    },
                    style = MaterialTheme.typography.bodyMedium,
                    color = Color.White.copy(alpha = 0.8f),
                )
            }
            Row(
                modifier = Modifier.align(Alignment.TopEnd).offset(y = (-20).dp),
            ) {
                IconButton(
                    onClick = onRefresh,
                    enabled = !isRefreshing,
                ) {
                    if (isRefreshing) {
                        CircularProgressIndicator(
                            Modifier.size(18.dp),
                            color = Color.White,
                            strokeWidth = 2.dp,
                        )
                    } else {
                        Icon(
                            Icons.Rounded.Refresh,
                            "Refresh balance",
                            tint = Color.White,
                        )
                    }
                }
                IconButton(onClick = onToggleBalanceVisibility) {
                    Icon(
                        if (isBalanceVisible) Icons.Rounded.Visibility else Icons.Rounded.VisibilityOff,
                        if (isBalanceVisible) "Hide balance" else "Show balance",
                        tint = Color.White,
                    )
                }
            }
        }
    }
}

@Composable
private fun WalletAction(
    label: String,
    icon: ImageVector,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val spacing = LocalAppSpacing.current
    Surface(
        onClick = onClick,
        modifier = modifier.height(76.dp),
        shape = MaterialTheme.shapes.medium,
        color = MaterialTheme.colorScheme.surface,
        tonalElevation = 1.dp,
    ) {
        Row(
            modifier = Modifier.padding(spacing.md),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.Center,
        ) {
            Surface(shape = CircleShape, color = MaterialTheme.colorScheme.primaryContainer) {
                Icon(
                    icon,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.primary,
                    modifier = Modifier.padding(10.dp).size(20.dp),
                )
            }
            Spacer(Modifier.width(spacing.sm))
            Text(label, style = MaterialTheme.typography.labelLarge)
        }
    }
}

@Composable
private fun TransactionRow(
    transaction: WalletTransaction,
    usdPerTon: Double?,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val spacing = LocalAppSpacing.current
    val tint = if (transaction.isReceived) Success else MaterialTheme.colorScheme.primary
    val sign = if (transaction.isReceived) "+" else "−"
    val date = remember(transaction.timestamp) {
        DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT)
            .format(Date(transaction.timestamp * 1000L))
    }
    Surface(
        onClick = onClick,
        modifier = modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.medium,
        color = MaterialTheme.colorScheme.surface,
        tonalElevation = 1.dp,
    ) {
        Row(
            modifier = Modifier.padding(spacing.md),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Surface(shape = CircleShape, color = tint.copy(alpha = 0.12f)) {
                Icon(
                    if (transaction.isReceived) Icons.Rounded.SouthWest else Icons.AutoMirrored.Rounded.Send,
                    contentDescription = null,
                    tint = tint,
                    modifier = Modifier.padding(11.dp).size(20.dp),
                )
            }
            Spacer(Modifier.width(spacing.md))
            Column(Modifier.weight(1f)) {
                Text(
                    if (transaction.isReceived) "Received" else "Sent",
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.SemiBold,
                )
                Text(
                    transaction.counterparty?.compactAddress() ?: date,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            Column(horizontalAlignment = Alignment.End) {
                Text(
                    "$sign${transaction.amountGrams} TON",
                    style = MaterialTheme.typography.titleSmall,
                    color = tint,
                    fontWeight = FontWeight.SemiBold,
                )
                val usd = transaction.amountGrams.toDoubleOrNull()?.let { amount ->
                    usdPerTon?.let { amount * it }
                }
                Text(
                    usd?.let { "${'$'}${String.format(Locale.US, "%.2f", it)}" } ?: date,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

@Composable
private fun TransactionDetailScreen(
    transaction: WalletTransaction,
    usdPerTon: Double?,
    contentPadding: PaddingValues,
    modifier: Modifier = Modifier,
) {
    val spacing = LocalAppSpacing.current
    val context = LocalContext.current
    var detailsExpanded by rememberSaveable(transaction.id) { mutableStateOf(false) }
    val tint = if (transaction.isReceived) Success else MaterialTheme.colorScheme.primary
    val sign = if (transaction.isReceived) "+" else "−"
    val date = remember(transaction.timestamp) {
        DateFormat.getDateTimeInstance(DateFormat.LONG, DateFormat.SHORT)
            .format(Date(transaction.timestamp * 1000L))
    }
    val usdAmount = transaction.amountGrams.toDoubleOrNull()?.let { amount ->
        usdPerTon?.let { rate -> amount * rate }
    }
    val formattedUsd = when {
        usdAmount == null -> "${'$'}—"
        usdAmount > 0.0 && usdAmount < 0.01 -> "< ${'$'}0.01"
        else -> "${'$'}${String.format(Locale.US, "%,.2f", usdAmount)}"
    }
    val counterparty = transaction.counterparty?.takeIf { it.isNotBlank() }
    val listPadding = PaddingValues(
        start = spacing.lg,
        end = spacing.lg,
        top = contentPadding.calculateTopPadding() + spacing.sm,
        bottom = contentPadding.calculateBottomPadding() + spacing.xl,
    )

    LazyColumn(
        modifier = modifier.fillMaxSize(),
        contentPadding = listPadding,
        verticalArrangement = Arrangement.spacedBy(spacing.lg),
    ) {
        item(key = "summary") {
            Column(
                modifier = Modifier.fillMaxWidth(),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(spacing.sm),
            ) {
                Surface(shape = CircleShape, color = tint.copy(alpha = 0.12f)) {
                    Icon(
                        if (transaction.isReceived) Icons.Rounded.SouthWest
                        else Icons.AutoMirrored.Rounded.Send,
                        contentDescription = null,
                        tint = tint,
                        modifier = Modifier.padding(16.dp).size(28.dp),
                    )
                }
                Text(
                    if (transaction.isReceived) "Received" else "Sent",
                    style = MaterialTheme.typography.titleLarge,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Text(
                    "$sign$formattedUsd",
                    style = MaterialTheme.typography.headlineLarge,
                    fontWeight = FontWeight.SemiBold,
                )
                Text(
                    "${transaction.amountGrams} TON",
                    fontFamily = FontFamily.Monospace,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Surface(shape = CircleShape, color = Success.copy(alpha = 0.12f)) {
                    Row(
                        modifier = Modifier.padding(horizontal = 12.dp, vertical = 6.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Icon(
                            Icons.Rounded.CheckCircle,
                            contentDescription = null,
                            tint = Success,
                            modifier = Modifier.size(18.dp),
                        )
                        Spacer(Modifier.width(spacing.xs))
                        Text("Confirmed", color = Success, style = MaterialTheme.typography.labelLarge)
                    }
                }
            }
        }
        item(key = "counterparty") {
            Surface(
                shape = MaterialTheme.shapes.medium,
                color = MaterialTheme.colorScheme.surface,
                tonalElevation = 1.dp,
            ) {
                Column(Modifier.padding(horizontal = spacing.md)) {
                    TransactionValueRow(title = "Date", value = date)
                    HorizontalDivider()
                    TransactionValueRow(
                        title = if (transaction.isReceived) "From" else "To",
                        value = counterparty?.compactTransactionAddress() ?: "Unknown address",
                        monospaced = true,
                        onCopy = counterparty?.let { address ->
                            { context.copyToClipboard("TON address", address) }
                        },
                    )
                }
            }
        }
        item(key = "technical-details") {
            Surface(
                onClick = { detailsExpanded = !detailsExpanded },
                shape = MaterialTheme.shapes.medium,
                color = MaterialTheme.colorScheme.surface,
                tonalElevation = 1.dp,
            ) {
                Column(Modifier.padding(horizontal = spacing.md)) {
                    Row(
                        modifier = Modifier.fillMaxWidth().padding(vertical = spacing.md),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(
                            "Transaction details",
                            style = MaterialTheme.typography.titleMedium,
                            modifier = Modifier.weight(1f),
                        )
                        Icon(
                            if (detailsExpanded) Icons.Rounded.KeyboardArrowUp
                            else Icons.Rounded.KeyboardArrowDown,
                            contentDescription = if (detailsExpanded) "Hide details" else "Show details",
                        )
                    }
                    AnimatedVisibility(visible = detailsExpanded) {
                        Column {
                            HorizontalDivider()
                            TransactionValueRow(
                                title = "Transaction ID",
                                value = transaction.transactionHash,
                                monospaced = true,
                            )
                            HorizontalDivider()
                            TransactionValueRow(
                                title = "Logical time",
                                value = transaction.logicalTime,
                                monospaced = true,
                            )
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun TransactionValueRow(
    title: String,
    value: String,
    monospaced: Boolean = false,
    onCopy: (() -> Unit)? = null,
) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(vertical = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(
            title,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.weight(0.38f),
        )
        SelectionContainer(Modifier.weight(0.62f)) {
            Text(
                value,
                fontFamily = if (monospaced) FontFamily.Monospace else FontFamily.Default,
                textAlign = TextAlign.End,
                maxLines = 1,
                overflow = TextOverflow.MiddleEllipsis,
                modifier = Modifier.fillMaxWidth(),
            )
        }
        if (onCopy != null) {
            IconButton(onClick = onCopy, modifier = Modifier.size(40.dp)) {
                Icon(Icons.Rounded.ContentCopy, contentDescription = "Copy address")
            }
        }
    }
}

@Composable
private fun EmptyActivity(modifier: Modifier = Modifier) {
    val spacing = LocalAppSpacing.current
    Column(
        modifier = modifier.fillMaxWidth().padding(vertical = spacing.xl),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Icon(
            Icons.Rounded.Wifi,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(spacing.sm))
        Text("No transactions yet", style = MaterialTheme.typography.titleMedium)
        Text(
            "Fund this address on testnet to get started.",
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            style = MaterialTheme.typography.bodyMedium,
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun CreateWalletSheet(onDismiss: () -> Unit, onCreate: (String) -> Unit) {
    var name by rememberSaveable { mutableStateOf("My Wallet") }
    WalletFormSheet(title = "Create wallet", onDismiss = onDismiss) {
        WalletSecurityIntro()
        OutlinedTextField(
            value = name,
            onValueChange = { name = it.take(32) },
            label = { Text("Wallet name") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
        Button(
            onClick = { onCreate(name) },
            enabled = name.isNotBlank(),
            modifier = Modifier.fillMaxWidth().height(54.dp),
        ) {
            Text("Generate recovery phrase")
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ImportWalletSheet(
    onDismiss: () -> Unit,
    onImport: (String, String) -> Unit,
) {
    SecureScreenEffect()
    var name by rememberSaveable { mutableStateOf("Imported Wallet") }
    var mnemonic by rememberSaveable { mutableStateOf("") }
    val wordCount = mnemonic.trim().split(Regex("\\s+")).count { it.isNotBlank() }
    WalletFormSheet(title = "Import wallet", onDismiss = onDismiss) {
        Text(
            "Enter the 12 or 24 words in the original order. The phrase is validated by the Rust core and never leaves this device.",
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        OutlinedTextField(
            value = name,
            onValueChange = { name = it.take(32) },
            label = { Text("Wallet name") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
        OutlinedTextField(
            value = mnemonic,
            onValueChange = { mnemonic = it },
            label = { Text("Recovery phrase") },
            supportingText = { Text("$wordCount words (12 or 24 required)") },
            minLines = 5,
            modifier = Modifier.fillMaxWidth(),
        )
        Button(
            onClick = {
                onImport(name, mnemonic)
                onDismiss()
            },
            enabled = name.isNotBlank() && (wordCount == 12 || wordCount == 24),
            modifier = Modifier.fillMaxWidth().height(54.dp),
        ) {
            Text("Import wallet")
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun WalletFormSheet(
    title: String,
    onDismiss: () -> Unit,
    content: @Composable ColumnScope.() -> Unit,
) {
    val spacing = LocalAppSpacing.current
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = spacing.lg, vertical = spacing.md),
            verticalArrangement = Arrangement.spacedBy(spacing.md),
        ) {
            Text(
                title,
                style = MaterialTheme.typography.headlineSmall,
                fontWeight = FontWeight.Bold,
                modifier = Modifier.semantics { heading() },
            )
            content()
            Spacer(Modifier.height(spacing.md))
        }
    }
}

@Composable
private fun WalletSecurityIntro(modifier: Modifier = Modifier) {
    val spacing = LocalAppSpacing.current
    Column(modifier = modifier, verticalArrangement = Arrangement.spacedBy(spacing.sm)) {
        SecurityLine(Icons.Rounded.Shield, "12 words generated with a cryptographic RNG")
        SecurityLine(Icons.Rounded.Lock, "Encrypted with Android Keystore")
        SecurityLine(Icons.Rounded.WifiOff, "Recovery phrase is never sent over the network")
    }
}

@Composable
private fun SecurityLine(icon: ImageVector, text: String, modifier: Modifier = Modifier) {
    val spacing = LocalAppSpacing.current
    Row(modifier = modifier, verticalAlignment = Alignment.CenterVertically) {
        Icon(icon, contentDescription = null, tint = Success, modifier = Modifier.size(22.dp))
        Spacer(Modifier.width(spacing.sm))
        Text(text, style = MaterialTheme.typography.bodyMedium)
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun RecoveryPhraseSheet(
    wallet: StoredWallet?,
    phrase: String,
    onDone: () -> Unit,
) {
    SecureScreenEffect()
    var confirmed by rememberSaveable { mutableStateOf(false) }
    val words = remember(phrase) { phrase.split(" ") }
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    ModalBottomSheet(
        onDismissRequest = { if (confirmed) onDone() },
        sheetState = sheetState,
    ) {
        val spacing = LocalAppSpacing.current
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = spacing.md, vertical = spacing.sm),
            verticalArrangement = Arrangement.spacedBy(spacing.md),
        ) {
            Text(
                "Save your recovery phrase",
                style = MaterialTheme.typography.headlineSmall,
                fontWeight = FontWeight.Bold,
                modifier = Modifier.semantics { heading() },
            )
            Text(
                "These words are the only way to restore ${wallet?.name ?: "this wallet"}. Never share them.",
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            words.chunked(3).forEachIndexed { rowIndex, rowWords ->
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(spacing.sm),
                ) {
                    rowWords.forEachIndexed { columnIndex, word ->
                        Surface(
                            modifier = Modifier.weight(1f),
                            shape = MaterialTheme.shapes.small,
                            color = MaterialTheme.colorScheme.surfaceVariant,
                        ) {
                            Row(Modifier.padding(10.dp), verticalAlignment = Alignment.CenterVertically) {
                                Text(
                                    "${rowIndex * 3 + columnIndex + 1}",
                                    style = MaterialTheme.typography.labelSmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    modifier = Modifier.width(22.dp),
                                )
                                Text(word, fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.bodySmall)
                            }
                        }
                    }
                }
            }
            Surface(
                onClick = { confirmed = !confirmed },
                shape = MaterialTheme.shapes.small,
                color = if (confirmed) MaterialTheme.colorScheme.primaryContainer
                    else MaterialTheme.colorScheme.surfaceVariant,
            ) {
                Row(
                    Modifier.fillMaxWidth().padding(spacing.md),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Icon(
                        if (confirmed) Icons.Rounded.Check else Icons.Rounded.Warning,
                        contentDescription = null,
                        tint = if (confirmed) MaterialTheme.colorScheme.primary else Warning,
                    )
                    Spacer(Modifier.width(spacing.sm))
                    Text("I saved these 12 words somewhere safe")
                }
            }
            Button(
                onClick = onDone,
                enabled = confirmed,
                modifier = Modifier.fillMaxWidth().height(54.dp),
            ) {
                Text("Open wallet")
            }
            Spacer(Modifier.height(spacing.md))
        }
    }
}

@Composable
private fun SecureScreenEffect() {
    val activity = LocalActivity.current
    DisposableEffect(activity) {
        activity?.window?.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        onDispose {
            activity?.window?.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SendSheet(
    state: WalletUiState,
    onDismiss: () -> Unit,
    onSend: (String, String, Boolean) -> Unit,
    onInputChanged: () -> Unit,
) {
    var destination by rememberSaveable { mutableStateOf("") }
    var amount by rememberSaveable { mutableStateOf("") }
    var force by rememberSaveable { mutableStateOf(false) }
    var isConfirming by rememberSaveable { mutableStateOf(false) }
    LaunchedEffect(state.canForceRetry) {
        if (!state.canForceRetry) force = false
    }
    WalletFormSheet(
        title = "Send GRAM",
        onDismiss = { if (!state.isSending) onDismiss() },
    ) {
        OutlinedTextField(
            value = destination,
            onValueChange = {
                destination = it
                force = false
                onInputChanged()
            },
            label = { Text("Recipient address") },
            minLines = 2,
            modifier = Modifier.fillMaxWidth(),
        )
        OutlinedTextField(
            value = amount,
            onValueChange = { value ->
                if (value.matches(Regex("[0-9]*([.][0-9]{0,9})?"))) {
                    amount = value
                    force = false
                    onInputChanged()
                }
            },
            label = { Text("Amount") },
            suffix = { Text("GRAM") },
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal),
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
        Text(
            "Available: ${state.account?.balanceGrams ?: "—"} GRAM. Network fees are charged separately.",
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            style = MaterialTheme.typography.bodySmall,
        )
        if (state.sendError != null) {
            SelectionContainer {
                Text(
                    state.sendError,
                    color = MaterialTheme.colorScheme.error,
                    style = MaterialTheme.typography.bodySmall,
                )
            }
        }
        if (state.canForceRetry) {
            Surface(
                color = MaterialTheme.colorScheme.errorContainer,
                contentColor = MaterialTheme.colorScheme.onErrorContainer,
                shape = MaterialTheme.shapes.medium,
            ) {
                Column(Modifier.fillMaxWidth().padding(LocalAppSpacing.current.md)) {
                    Text(
                        "Previous transfer is unresolved",
                        style = MaterialTheme.typography.titleSmall,
                    )
                    Text(
                        "Its signed message may still execute. Submitting again authorizes another message for the wallet's current sequence number; network ordering determines which can execute.",
                        style = MaterialTheme.typography.bodySmall,
                        modifier = Modifier.padding(top = LocalAppSpacing.current.xs),
                    )
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        modifier = Modifier
                            .fillMaxWidth()
                            .clickable(enabled = !state.isSending) { force = !force }
                            .padding(top = LocalAppSpacing.current.sm),
                    ) {
                        Checkbox(
                            checked = force,
                            enabled = !state.isSending,
                            onCheckedChange = { force = it },
                        )
                        Text(
                            "I understand. Submit this transfer anyway.",
                            style = MaterialTheme.typography.bodySmall,
                        )
                    }
                }
            }
        }
        Button(
            onClick = { isConfirming = true },
            enabled = !state.isSending &&
                destination.isNotBlank() &&
                (amount.toDoubleOrNull() ?: 0.0) > 0 &&
                (!state.canForceRetry || force),
            modifier = Modifier.fillMaxWidth().height(54.dp),
        ) {
            if (state.isSending) {
                CircularProgressIndicator(Modifier.size(18.dp), color = Color.White, strokeWidth = 2.dp)
                Spacer(Modifier.width(LocalAppSpacing.current.sm))
                Text("Signing…")
            } else {
                Icon(Icons.AutoMirrored.Rounded.Send, contentDescription = null)
                Spacer(Modifier.width(LocalAppSpacing.current.sm))
                Text("Send")
            }
        }
    }
    if (isConfirming) {
        AlertDialog(
            onDismissRequest = { if (!state.isSending) isConfirming = false },
            title = { Text("Confirm transfer") },
            text = {
                Text(
                    buildString {
                        append("Send ${amount.trim()} GRAM to ${destination.trim()}?")
                        if (force) {
                            append("\n\nThe previous signed transfer may still execute, so both transfers can affect the balance.")
                        }
                    },
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        isConfirming = false
                        onSend(destination.trim(), amount.trim(), force)
                    },
                ) { Text("Send") }
            },
            dismissButton = {
                TextButton(onClick = { isConfirming = false }) { Text("Cancel") }
            },
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ReceiveSheet(wallet: StoredWallet, onDismiss: () -> Unit) {
    val context = LocalContext.current
    WalletFormSheet(title = "Receive TON", onDismiss = onDismiss) {
        Surface(
            shape = MaterialTheme.shapes.medium,
            color = MaterialTheme.colorScheme.surfaceVariant,
        ) {
            Column(Modifier.padding(LocalAppSpacing.current.md)) {
                Text("Your wallet address", style = MaterialTheme.typography.labelMedium)
                Spacer(Modifier.height(LocalAppSpacing.current.sm))
                Text(
                    wallet.address,
                    fontFamily = FontFamily.Monospace,
                    style = MaterialTheme.typography.bodyMedium,
                )
            }
        }
        Button(
            onClick = { context.copyToClipboard("TON address", wallet.address) },
            modifier = Modifier.fillMaxWidth().height(54.dp),
        ) {
            Icon(Icons.Rounded.ContentCopy, contentDescription = null)
            Spacer(Modifier.width(LocalAppSpacing.current.sm))
            Text("Copy address")
        }
        Text(
            "Only send testnet TON to this address.",
            color = Warning,
            style = MaterialTheme.typography.bodySmall,
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SettingsSheet(
    state: WalletUiState,
    onDismiss: () -> Unit,
    onRename: (String) -> Unit,
    onDelete: () -> Unit,
    onImport: () -> Unit,
) {
    var name by rememberSaveable(state.activeWallet?.address) {
        mutableStateOf(state.activeWallet?.name.orEmpty())
    }
    var confirmDelete by remember { mutableStateOf(false) }
    WalletFormSheet(title = "Wallet settings", onDismiss = onDismiss) {
        Text(
            state.activeWallet?.address?.compactAddress().orEmpty(),
            fontFamily = FontFamily.Monospace,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        OutlinedTextField(
            value = name,
            onValueChange = { name = it.take(32) },
            label = { Text("Wallet name") },
            singleLine = true,
            trailingIcon = {
                IconButton(onClick = { onRename(name) }, enabled = name.isNotBlank()) {
                    Icon(Icons.Rounded.Edit, contentDescription = "Save wallet name")
                }
            },
            modifier = Modifier.fillMaxWidth(),
        )
        OutlinedButton(onClick = onImport, modifier = Modifier.fillMaxWidth()) {
            Icon(Icons.Rounded.Add, contentDescription = null)
            Spacer(Modifier.width(LocalAppSpacing.current.sm))
            Text("Import another wallet")
        }
        HorizontalDivider()
        TextButton(
            onClick = { confirmDelete = true },
            colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error),
            modifier = Modifier.fillMaxWidth(),
        ) {
            Icon(Icons.Rounded.DeleteOutline, contentDescription = null)
            Spacer(Modifier.width(LocalAppSpacing.current.sm))
            Text("Remove wallet from this device")
        }
    }
    if (confirmDelete) {
        AlertDialog(
            onDismissRequest = { confirmDelete = false },
            icon = { Icon(Icons.Rounded.DeleteOutline, contentDescription = null) },
            title = { Text("Remove ${state.activeWallet?.name}?") },
            text = { Text("The encrypted recovery phrase and local wallet data will be deleted. This cannot be undone.") },
            confirmButton = {
                TextButton(onClick = onDelete) { Text("Remove") }
            },
            dismissButton = {
                TextButton(onClick = { confirmDelete = false }) { Text("Cancel") }
            },
        )
    }
}

private fun String.compactAddress(): String =
    if (length <= 16) this else "${take(8)}…${takeLast(6)}"

private fun String.compactTransactionAddress(): String =
    if (length <= 15) this else "${take(6)}…${takeLast(6)}"

private fun Context.copyToClipboard(label: String, value: String) {
    val clipboard = getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
    clipboard.setPrimaryClip(ClipData.newPlainText(label, value))
}
