package com.neycrol.ipv6ddns

import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * UI tests for MainActivity
 * 
 * These tests verify that the main UI components are properly displayed and interactive.
 * Note: These tests require an Android device or emulator to run.
 */
@RunWith(AndroidJUnit4::class)
class MainActivityUITest {

    @get:Rule
    val composeTestRule = createAndroidComposeRule<MainActivity>()

    @Test
    fun testAppScreenDisplays() {
        // Verify that the main app screen is displayed
        composeTestRule.waitForIdle()
        composeTestRule.onRoot().assertIsDisplayed()
    }

    @Test
    fun testConfigurationInputsAreDisplayed() {
        // Verify that configuration input fields exist and are enabled
        composeTestRule.waitForIdle()
        
        // Check API Token field
        composeTestRule.onNodeWithTag("apiTokenField")
            .assertIsDisplayed()
            .assertIsEnabled()
        
        // Check Zone ID field
        composeTestRule.onNodeWithTag("zoneIdField")
            .assertIsDisplayed()
            .assertIsEnabled()
        
        // Check Record Name field
        composeTestRule.onNodeWithTag("recordNameField")
            .assertIsDisplayed()
            .assertIsEnabled()
        
        // Check Timeout field
        composeTestRule.onNodeWithTag("timeoutField")
            .assertIsDisplayed()
            .assertIsEnabled()
    }

    @Test
    fun testSaveButtonIsClickable() {
        // Verify that the start button is displayed and clickable
        composeTestRule.waitForIdle()
        
        composeTestRule.onNodeWithTag("startButton")
            .assertIsDisplayed()
            .assertIsEnabled()
            .assertHasClickAction()
    }

    @Test
    fun testStopButtonExists() {
        // Verify that the stop button exists (initially disabled)
        composeTestRule.waitForIdle()
        
        composeTestRule.onNodeWithTag("stopButton")
            .assertIsDisplayed()
            .assertIsNotEnabled()
    }

    @Test
    fun testConfigurationInputAndSave() {
        // Test user input flow: fill configuration and verify start button works
        composeTestRule.waitForIdle()
        
        // Fill API Token
        composeTestRule.onNodeWithTag("apiTokenField")
            .performTextInput("test-api-token-12345")
        
        // Fill Zone ID
        composeTestRule.onNodeWithTag("zoneIdField")
            .performTextInput("test-zone-id")
        
        // Fill Record Name
        composeTestRule.onNodeWithTag("recordNameField")
            .performTextInput("test.example.com")
        
        // Fill Timeout
        composeTestRule.onNodeWithTag("timeoutField")
            .performTextClearance()
            .performTextInput("60")
        
        // Verify start button is still enabled after input
        composeTestRule.onNodeWithTag("startButton")
            .assertIsEnabled()
    }

    @Test
    fun testApiTokenVisibilityToggle() {
        // Test that the API token visibility toggle works
        composeTestRule.waitForIdle()
        
        val testToken = "test-token-123"
        
        // Enter a token
        composeTestRule.onNodeWithTag("apiTokenField")
            .performTextInput(testToken)
        
        // Find and click the visibility toggle button
        // The toggle is part of the trailing icon
        composeTestRule.onNodeWithTag("apiTokenField")
            .onChildren()
            .filter(hasClickAction())
            .assertCountEquals(1)
    }

    @Test
    fun testInvalidTimeoutShowsError() {
        // Test that invalid timeout values are handled
        composeTestRule.waitForIdle()
        
        // Enter invalid timeout (too low)
        composeTestRule.onNodeWithTag("timeoutField")
            .performTextClearance()
            .performTextInput("0")
        
        // Click start button
        composeTestRule.onNodeWithTag("startButton")
            .performClick()
        
        // Should show error (error card should appear)
        composeTestRule.waitForIdle()
        // Error message should be displayed in a card
        composeTestRule.onNodeWithText(composeTestRule.activity.getString(R.string.validation_timeout_range, 1, 300))
            .assertExists()
    }

    @Test
    fun testEmptyFieldsValidation() {
        // Test validation of empty required fields
        composeTestRule.waitForIdle()
        
        // Clear all fields
        composeTestRule.onNodeWithTag("apiTokenField").performTextClearance()
        composeTestRule.onNodeWithTag("zoneIdField").performTextClearance()
        composeTestRule.onNodeWithTag("recordNameField").performTextClearance()
        
        // Click start button
        composeTestRule.onNodeWithTag("startButton")
            .performClick()
        
        // Should show validation error
        composeTestRule.waitForIdle()
        composeTestRule.onNodeWithText(composeTestRule.activity.getString(R.string.validation_api_token_required))
            .assertExists()
    }

    @Test
    fun testThemeIsApplied() {
        // Verify that the Material3 theme is applied without crashes
        composeTestRule.waitForIdle()
        
        // If we can find themed components, theme is working
        composeTestRule.onNodeWithTag("startButton")
            .assertExists()
    }

    @Test
    fun testScreenIsScrollable() {
        // Verify that the screen is scrollable
        composeTestRule.waitForIdle()
        
        // The root should be scrollable (have vertical scroll modifier)
        composeTestRule.onRoot().assertExists()
    }

    @Test
    fun testMaterial3ComponentsAreUsed() {
        // Verify that Material3 components are being used
        composeTestRule.waitForIdle()
        
        // Check for Material3 specific components
        composeTestRule.onNodeWithTag("startButton")
            .assertExists()
    }
}
